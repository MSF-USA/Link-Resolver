"""Exercise macOS staging and validation without requiring Apple's build tools."""

import hashlib
import importlib.util
import pathlib
import plistlib
import shutil
import struct
import subprocess
import tempfile
import unittest
from unittest import mock
import zipfile
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
SPEC = importlib.util.spec_from_file_location('package_macos', ROOT / 'packaging/package-macos.py')
packager = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(packager)


class MacOSPackagingTests(unittest.TestCase):
    """Check real archive contents; mock only macOS tools and native compilation."""

    def setUp(self):
        """Create an isolated project with executable fixtures and release-stamped metadata."""
        self.scratch = tempfile.TemporaryDirectory(prefix='package-macos-')
        self.addCleanup(self.scratch.cleanup)
        self.root = pathlib.Path(self.scratch.name)
        (self.root / 'Cargo.toml').write_text(
            '[package]\nversion = "1.20261110.2"\n', encoding='utf-8',
        )
        for name in ('QUICKSTART.txt', 'packaging/QUICKSTART-terminal.txt',
                     'examples/urls.txt', 'examples/links.csv'):
            destination = self.root / name
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(ROOT / name, destination)
        self.calls = []

    def tool(self, arguments, root, env=None):
        """Simulate native compilation and inspection, recording calls for assertions."""
        self.assertEqual(root, self.root)
        self.calls.append(arguments)
        if arguments[0] == 'cargo':
            self.assertEqual(env['MACOSX_DEPLOYMENT_TARGET'], '13.0')
            self.assertIn('--locked', arguments)
            target = arguments[arguments.index('--target') + 1]
            out = self.root / 'target' / target / 'release'
            out.mkdir(parents=True, exist_ok=True)
            for name in ('link-resolver', 'link-resolver-cli'):
                binary = out / name
                binary.write_bytes(b'fixture executable\n')
                binary.chmod(0o755)
        elif arguments[0] == 'otool':
            if arguments[1] == '-L':
                return 'fixture:\n\t/usr/lib/libSystem.B.dylib (compatibility version 1.0.0)'
            return 'Load command 1\n      cmd LC_BUILD_VERSION\n  cmdsize 32\n platform 1\n    minos 13.0\n      sdk 15.0\n'
        elif arguments[-1] == '--version':
            return 'Link Resolver 1.20261110.2'
        return ''

    def build(self, target='aarch64-apple-darwin'):
        """Run real packaging with fake native tools; return the resulting archive paths."""
        with mock.patch.object(packager.sys, 'platform', 'darwin'), \
                mock.patch.object(packager.shutil, 'which', return_value='/fake/tool'), \
                mock.patch.object(packager, 'run', side_effect=self.tool), \
                mock.patch.object(packager, 'icon_bytes', return_value=b'fixture icon'):
            return packager.package(self.root, target, '13.0')

    def test_app_cli_archives_metadata_permissions_and_checksums(self):
        """Ship a Finder bundle with matching metadata, executable CLIs and correct hashes."""
        archives = self.build()
        self.assertEqual([path.name for path in archives], [
            'link-resolver-gui-macos-arm64.zip', 'link-resolver-macos-arm64.zip',
        ])
        with zipfile.ZipFile(archives[0]) as archive:
            prefix = 'link-resolver/Link Resolver.app/Contents/'
            metadata = plistlib.loads(archive.read(prefix + 'Info.plist'))
            self.assertEqual(metadata['CFBundleExecutable'], 'link-resolver')
            self.assertEqual(metadata['CFBundleIdentifier'], packager.BUNDLE_IDENTIFIER)
            self.assertEqual(metadata['CFBundleShortVersionString'], '1.20261110.2')
            self.assertEqual(metadata['CFBundleVersion'], packager.bundle_build_version('1.20261110.2'))
            self.assertEqual(metadata['LSMinimumSystemVersion'], '13.0')
            self.assertTrue(metadata['NSHighResolutionCapable'])
            self.assertEqual(archive.read(prefix + 'Resources/link-resolver.icns'), b'fixture icon')
            for name in (prefix + 'MacOS/link-resolver', 'link-resolver/link-resolver-cli'):
                self.assertEqual(archive.getinfo(name).external_attr >> 16 & 0o777, 0o755)
            self.assertIn('link-resolver/urls.txt', archive.namelist())
            self.assertIn('link-resolver/links.csv', archive.namelist())
        with zipfile.ZipFile(archives[1]) as archive:
            self.assertIn('link-resolver/link-resolver', archive.namelist())
            self.assertNotIn('link-resolver/link-resolver-cli', archive.namelist())
            self.assertFalse(any('.app/' in name for name in archive.namelist()))
            self.assertIn(b'terminal edition', archive.read('link-resolver/QUICKSTART.txt'))
        checksums = (self.root / 'dist/SHA256SUMS-macos-arm64').read_text()
        for path in archives:
            self.assertIn(f'{hashlib.sha256(path.read_bytes()).hexdigest()}  {path.name}\n', checksums)
        signed = [call[-1] for call in self.calls if call[:3] == ['codesign', '--force', '--sign']]
        self.assertEqual(len(signed), 3)
        self.assertTrue(any(path.endswith('Link Resolver.app') for path in signed))
        self.assertFalse(list((self.root / 'target').glob('macos-staging-*')))

    def test_intel_option_preserves_unrelated_distribution_files(self):
        """An explicit Intel build uses x64 names and never deletes other platforms' files."""
        dist = self.root / 'dist'
        dist.mkdir()
        other = dist / 'link-resolver-gui-linux-x64.tar.gz'
        other.write_bytes(b'existing archive')
        archives = self.build('x86_64-apple-darwin')
        self.assertTrue(all(path.name.endswith('macos-x64.zip') for path in archives))
        self.assertEqual(other.read_bytes(), b'existing archive')
        self.assertIn(['lipo', '-verify_arch', 'x86_64',
                       str(self.root / 'target/x86_64-apple-darwin/release/link-resolver')], self.calls)

    def test_invalid_platform_target_or_baseline_fails_before_building(self):
        """Invalid options and non-Mac hosts cannot start tools or write distribution files."""
        with mock.patch.object(packager, 'run') as tool:
            for target, baseline in (('invalid', '13.0'), ('aarch64-apple-darwin', '../13')):
                with self.subTest(target=target, baseline=baseline), self.assertRaises(ValueError):
                    packager.package(self.root, target, baseline)
            with mock.patch.object(packager.sys, 'platform', 'linux'), \
                    self.assertRaisesRegex(ValueError, 'requires a Mac'):
                packager.package(self.root, 'aarch64-apple-darwin', '13.0')
            tool.assert_not_called()
        self.assertFalse((self.root / 'dist').exists())

    def test_missing_tool_and_build_failure_are_not_ignored(self):
        """A missing tool or failed compiler aborts without emitting misleading archives."""
        with mock.patch.object(packager.sys, 'platform', 'darwin'), \
                mock.patch.object(packager.shutil, 'which', return_value=None), \
                self.assertRaisesRegex(ValueError, 'Missing required tool: cargo'):
            packager.package(self.root, 'aarch64-apple-darwin', '13.0')
        with mock.patch.object(packager.sys, 'platform', 'darwin'), \
                mock.patch.object(packager.shutil, 'which', return_value='/fake/tool'), \
                mock.patch.object(packager, 'run', side_effect=subprocess.CalledProcessError(1, 'cargo')), \
                self.assertRaises(subprocess.CalledProcessError):
            packager.package(self.root, 'aarch64-apple-darwin', '13.0')
        self.assertFalse((self.root / 'dist').exists())

    def test_version_mismatch_and_signing_failure_abort_packaging(self):
        """Neither stale binaries nor invalid signatures may become a successful release."""
        def faulty_tool(arguments, root, env=None):
            output = self.tool(arguments, root, env)
            if arguments[-1] == '--version':
                return 'Link Resolver 0.1.0'
            return output

        with mock.patch.object(packager.sys, 'platform', 'darwin'), \
                mock.patch.object(packager.shutil, 'which', return_value='/fake/tool'), \
                mock.patch.object(packager, 'run', side_effect=faulty_tool), \
                self.assertRaisesRegex(ValueError, 'CLI version mismatch'):
            packager.package(self.root, 'aarch64-apple-darwin', '13.0')
        self.assertFalse((self.root / 'dist').exists())

        def failed_signature(arguments, root, env=None):
            if arguments[:2] == ['codesign', '--verify']:
                raise subprocess.CalledProcessError(1, arguments, stderr='invalid signature')
            return self.tool(arguments, root, env)

        with mock.patch.object(packager.sys, 'platform', 'darwin'), \
                mock.patch.object(packager.shutil, 'which', return_value='/fake/tool'), \
                mock.patch.object(packager, 'run', side_effect=failed_signature), \
                mock.patch.object(packager, 'icon_bytes', return_value=b'fixture icon'), \
                self.assertRaises(subprocess.CalledProcessError):
            packager.package(self.root, 'aarch64-apple-darwin', '13.0')
        self.assertFalse(list((self.root / 'dist').glob('*.zip')))
        self.assertFalse(list((self.root / 'target').glob('macos-staging-*')))

    def test_macho_baseline_brackets_boundary_and_system_dependencies(self):
        """Accept equal/older OS versions, reject newer ones and unbundled dependencies."""
        binary = self.root / 'fixture'
        for version, accepted in (('12.0', True), ('13.0', True), ('13.0.0', True), ('13.1', False)):
            with self.subTest(version=version), mock.patch.object(packager, 'run', side_effect=[
                '', 'fixture:\n\t/System/Library/Frameworks/AppKit.framework/AppKit (compatibility version 1)',
                f'Load command 1\n cmd LC_BUILD_VERSION\n minos {version}\n sdk 15.0',
            ]):
                if accepted:
                    packager.check_binary(binary, 'arm64', '13.0', self.root)
                else:
                    with self.assertRaisesRegex(ValueError, 'above 13.0'):
                        packager.check_binary(binary, 'arm64', '13.0', self.root)
        with mock.patch.object(packager, 'run', side_effect=[
            '', 'fixture:\n\t/opt/homebrew/lib/libexample.dylib (compatibility version 1)',
        ]), self.assertRaisesRegex(ValueError, 'non-system library'):
            packager.check_binary(binary, 'arm64', '13.0', self.root)
        with mock.patch.object(packager, 'run', side_effect=['', 'fixture:', '']), \
                self.assertRaisesRegex(ValueError, 'no macOS deployment version'):
            packager.check_binary(binary, 'arm64', '13.0', self.root)

    def test_icon_contains_valid_pngs_at_all_declared_sizes(self):
        """The generated ICNS has decodable PNG payloads, alpha and the brand red pixels."""
        data = packager.icon_bytes()
        self.assertEqual(data[:4], b'icns')
        self.assertEqual(struct.unpack('>I', data[4:8])[0], len(data))
        position = 8
        for kind, size in ((b'ic08', 256), (b'ic09', 512), (b'ic10', 1024)):
            self.assertEqual(data[position:position + 4], kind)
            length = struct.unpack('>I', data[position + 4:position + 8])[0]
            png = data[position + 8:position + length]
            self.assertEqual(png[:8], b'\x89PNG\r\n\x1a\n')
            self.assertEqual(struct.unpack('>II', png[16:24]), (size, size))
            offset = 8
            compressed = b''
            while offset < len(png):
                chunk_length = struct.unpack('>I', png[offset:offset + 4])[0]
                chunk = png[offset + 4:offset + 8 + chunk_length]
                crc = struct.unpack('>I', png[offset + 8 + chunk_length:offset + 12 + chunk_length])[0]
                self.assertEqual(zlib.crc32(chunk) & 0xffffffff, crc)
                if chunk[:4] == b'IDAT':
                    compressed += chunk[4:]
                offset += chunk_length + 12
            pixels = zlib.decompress(compressed)
            self.assertEqual(len(pixels), size * (1 + 4 * size))
            self.assertIn(bytes((238, 0, 0, 255)), pixels)
            self.assertEqual(pixels[1:5], b'\x00\x00\x00\x00')
            position += length
        self.assertEqual(position, len(data))

    def test_bundle_build_versions_fit_apple_limits_and_sort_by_release(self):
        """Date-based release versions stay monotonic without exceeding Apple's digit limits."""
        self.assertEqual(packager.bundle_build_version('0.1.0'), '1.1.0')
        versions = [packager.bundle_build_version(version) for version in (
            '1.20261231.0', '1.20261231.2', '1.20261231.99', '1.20270101.0',
        )]
        numbers = [tuple(map(int, version.split('.'))) for version in versions]
        self.assertEqual(numbers, sorted(numbers))
        self.assertEqual(len(set(numbers)), len(numbers))
        for first, second, third in numbers:
            self.assertTrue(1 <= first <= 9999)
            self.assertTrue(0 <= second <= 99)
            self.assertTrue(0 <= third <= 99)
        for invalid in ('latest', '1.20260230.0', '1.20261110.100', '9999.0.0'):
            with self.subTest(version=invalid), self.assertRaises(ValueError):
                packager.bundle_build_version(invalid)
