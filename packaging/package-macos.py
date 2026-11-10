#!/usr/bin/env python3
"""Build a macOS app + CLI ZIP and a terminal-only ZIP using native Apple tools.

Requires macOS, Rust 1.95+, Python 3.11+, and Xcode Command Line Tools.
Defaults to Apple Silicon and macOS 13.0; --target also supports Intel builds.
Uses ad-hoc signatures, not Developer ID signing or notarization. Other platforms'
archives are left alone. Run: python3 packaging/package-macos.py --help.
"""

import argparse
import datetime
import hashlib
import os
import pathlib
import plistlib
import re
import shutil
import stat
import struct
import subprocess
import sys
import tempfile
import tomllib
import zipfile
import zlib

ROOT = pathlib.Path(__file__).resolve().parents[1]
TARGETS = {
    'aarch64-apple-darwin': ('arm64', 'arm64'),
    'x86_64-apple-darwin': ('x64', 'x86_64'),
}
DEFAULT_DEPLOYMENT_TARGET = '13.0'
BUNDLE_IDENTIFIER = 'org.link-resolver.desktop'


def run(arguments, root, env=None):
    """Run a tool in root with optional environment; return stdout or raise on failure."""
    result = subprocess.run(
        arguments, cwd=root, env=env, check=True, text=True,
        stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    return result.stdout.strip()


def check_binary(binary, architecture, deployment_target, root):
    """Validate a Mach-O's CPU, OS baseline and system-only dependencies; raise ValueError."""
    run(['lipo', '-verify_arch', architecture, str(binary)], root)
    dependencies = run(['otool', '-L', str(binary)], root)
    for line in dependencies.splitlines()[1:]:
        library = line.strip().split(' (', 1)[0]
        if not library.startswith(('/System/Library/', '/usr/lib/')):
            raise ValueError(f'{binary} depends on a non-system library: {library}')
    commands = run(['otool', '-l', str(binary)], root)
    versions = re.findall(
        r'cmd LC_(?:BUILD_VERSION|VERSION_MIN_MACOSX)\b(?:(?!\nLoad command).)*?'
        r'\n\s*(?:minos|version)\s+(\d+(?:\.\d+)*)', commands, re.S,
    )
    if not versions:
        raise ValueError(f'{binary} has no macOS deployment version')
    baseline = tuple(int(part) for part in deployment_target.split('.'))
    for version in versions:
        measured = tuple(int(part) for part in version.split('.'))
        if (measured + (0,) * 3)[:3] > (baseline + (0,) * 3)[:3]:
            raise ValueError(f'{binary} needs macOS {version}, above {deployment_target}')


def icon_bytes():
    """Return a PNG-backed ICNS with a red chain on the GUI's neutral brand surface."""
    def chunk(kind, data):
        return struct.pack('>I', len(data)) + kind + data + struct.pack(
            '>I', zlib.crc32(kind + data) & 0xffffffff,
        )

    images = []
    for size, kind in ((256, b'ic08'), (512, b'ic09'), (1024, b'ic10')):
        pixels = bytearray()
        for row in range(size):
            pixels.append(0)  # PNG scanline with no filter.
            for column in range(size):
                x, y = (column + 0.5) / size, (row + 0.5) / size
                # Rounded square; links are two diagonal capsule outlines.
                corner_x = max(abs(x - 0.5) - 0.29, 0)
                corner_y = max(abs(y - 0.5) - 0.29, 0)
                inside = corner_x ** 2 + corner_y ** 2 <= 0.16 ** 2
                along = (x + y - 1) * 0.70710678
                across = (y - x) * 0.70710678
                linked = any(
                    0.057 ** 2 <= max(abs(along - center) - 0.09, 0) ** 2
                    + across ** 2 <= 0.105 ** 2
                    for center in (-0.105, 0.105)
                )
                color = (238, 0, 0, 255) if linked else (239, 240, 235, 255)
                pixels.extend(color if inside else (0, 0, 0, 0))
        png = b'\x89PNG\r\n\x1a\n'
        png += chunk(b'IHDR', struct.pack('>IIBBBBB', size, size, 8, 6, 0, 0, 0))
        png += chunk(b'IDAT', zlib.compress(pixels, 9)) + chunk(b'IEND', b'')
        images.append(kind + struct.pack('>I', len(png) + 8) + png)
    contents = b''.join(images)
    return b'icns' + struct.pack('>I', len(contents) + 8) + contents


def stage_app(folder, binaries, version, deployment_target):
    """Stage an app and CLI from two binaries; return the .app path, propagating I/O errors."""
    app = folder / 'Link Resolver.app'
    contents = app / 'Contents'
    (contents / 'MacOS').mkdir(parents=True)
    (contents / 'Resources').mkdir()
    shutil.copy2(binaries[0], contents / 'MacOS' / 'link-resolver')
    shutil.copy2(binaries[1], folder / 'link-resolver-cli')
    (contents / 'Resources' / 'link-resolver.icns').write_bytes(icon_bytes())
    metadata = {
        'CFBundleName': 'Link Resolver',
        'CFBundleDisplayName': 'Link Resolver',
        'CFBundleIdentifier': BUNDLE_IDENTIFIER,
        'CFBundleExecutable': 'link-resolver',
        'CFBundlePackageType': 'APPL',
        'CFBundleIconFile': 'link-resolver.icns',
        'CFBundleShortVersionString': version,
        'CFBundleVersion': bundle_build_version(version),
        'CFBundleInfoDictionaryVersion': '6.0',
        'CFBundleDevelopmentRegion': 'en',
        'LSMinimumSystemVersion': deployment_target,
        'NSHighResolutionCapable': True,
    }
    with (contents / 'Info.plist').open('wb') as output:
        plistlib.dump(metadata, output)
    return app


def bundle_build_version(version):
    """Map a three-part crate version to Apple's 4/2/2-digit build format or raise ValueError.

    For 1.YYYYMMDD.N, encode the date's ordinal in the first two base-100 fields
    and keep N (0..99) last. Other versions shift the major by one so development
    version 0.1.0 has a positive first field. The displayed version stays unchanged.
    """
    if not re.fullmatch(r'\d+\.\d+\.\d+', version):
        raise ValueError(f'Unsupported macOS bundle version: {version}')
    major, minor, patch = (int(part) for part in version.split('.'))
    if major == 1 and len(str(minor)) == 8:
        date = datetime.datetime.strptime(str(minor), '%Y%m%d').date()
        first, second = divmod(date.toordinal(), 100)
    else:
        first, second = major + 1, minor
    if not (1 <= first <= 9999 and 0 <= second <= 99 and 0 <= patch <= 99):
        raise ValueError(f'{version} exceeds Apple bundle build-version limits')
    return f'{first}.{second}.{patch}'


def write_archive(folder, destination):
    """Atomically write a ZIP under link-resolver/, preserving modes; propagate I/O errors."""
    temporary = destination.with_suffix('.zip.tmp')
    try:
        with zipfile.ZipFile(temporary, 'w', zipfile.ZIP_DEFLATED) as archive:
            for path in sorted(folder.rglob('*')):
                name = pathlib.Path('link-resolver') / path.relative_to(folder)
                archive.write(path, name)
                info = archive.getinfo(name.as_posix() + ('/' if path.is_dir() else ''))
                info.create_system = 3  # Unix modes, including in cross-platform fixture tests.
                if path.is_file() and path.name in ('link-resolver', 'link-resolver-cli'):
                    info.external_attr = (stat.S_IFREG | 0o755) << 16
        temporary.replace(destination)
    finally:
        temporary.unlink(missing_ok=True)


def package(root, target, deployment_target):
    """Build and check target, then write two ZIPs and macOS checksums; raise on failure.

    root is the project Path, target a key in TARGETS, deployment_target a numeric
    major.minor[.patch] macOS version. Requires Apple's native build/signing tools.
    Returns the two archive Paths. No URLs are contacted by the packaged CLI check.
    """
    if target not in TARGETS:
        raise ValueError(f'Unsupported macOS target: {target}')
    if not re.fullmatch(r'\d+\.\d+(?:\.\d+)?', deployment_target):
        raise ValueError('Deployment target must be major.minor[.patch], such as 13.0')
    if sys.platform != 'darwin':
        raise ValueError('macOS packaging requires a Mac with Xcode Command Line Tools')
    for tool in ('cargo', 'lipo', 'otool', 'codesign', 'plutil'):
        if shutil.which(tool) is None:
            raise ValueError(f'Missing required tool: {tool}')
    version = tomllib.loads((root / 'Cargo.toml').read_text(encoding='utf-8'))['package']['version']
    bundle_build_version(version)
    label, architecture = TARGETS[target]
    env = dict(os.environ, MACOSX_DEPLOYMENT_TARGET=deployment_target)
    print(f'Building {target} for macOS {deployment_target}+', flush=True)
    run(['cargo', 'build', '--release', '--locked', '--target', target,
         '--target-dir', str(root / 'target')], root, env)
    out = root / 'target' / target / 'release'
    binaries = (out / 'link-resolver', out / 'link-resolver-cli')
    for binary in binaries:
        check_binary(binary, architecture, deployment_target, root)
    reported = run([str(binaries[1]), '--version'], root)
    if reported != f'Link Resolver {version}':
        raise ValueError(f'CLI version mismatch: {reported!r}, expected Link Resolver {version}')

    dist = root / 'dist'
    dist.mkdir(exist_ok=True)
    archives = []
    with tempfile.TemporaryDirectory(prefix='macos-staging-', dir=root / 'target') as scratch:
        for gui in (True, False):
            folder = pathlib.Path(scratch) / ('gui' if gui else 'terminal')
            folder.mkdir()
            quickstart = root / ('QUICKSTART.txt' if gui else 'packaging/QUICKSTART-terminal.txt')
            shutil.copy2(quickstart, folder / 'QUICKSTART.txt')
            for name in ('urls.txt', 'links.csv'):
                shutil.copy2(root / 'examples' / name, folder / name)
            if gui:
                app = stage_app(folder, binaries, version, deployment_target)
                run(['plutil', '-lint', str(app / 'Contents' / 'Info.plist')], root)
                # Apple Silicon requires code signatures. Ad-hoc signatures don't
                # establish developer identity or satisfy downloaded-app Gatekeeper.
                run(['codesign', '--force', '--sign', '-', str(app)], root)
                run(['codesign', '--verify', '--deep', '--strict', str(app)], root)
                cli = folder / 'link-resolver-cli'
            else:
                cli = folder / 'link-resolver'
                shutil.copy2(binaries[1], cli)
            run(['codesign', '--force', '--sign', '-', str(cli)], root)
            run(['codesign', '--verify', '--strict', str(cli)], root)
            name = f'link-resolver-{"gui-" if gui else ""}macos-{label}.zip'
            destination = dist / name
            write_archive(folder, destination)
            archives.append(destination)
    checksum_lines = []
    for path in archives:
        with path.open('rb') as source:
            digest = hashlib.file_digest(source, 'sha256').hexdigest()
        checksum_lines.append(f'{digest}  {path.name}\n')
    sums = ''.join(checksum_lines)
    (dist / f'SHA256SUMS-macos-{label}').write_text(sums, encoding='utf-8')
    print(sums, end='')
    return archives


def main():
    """Parse packaging options and return 0 on success, 1 for build/validation/I/O errors."""
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--target', choices=TARGETS, default='aarch64-apple-darwin')
    parser.add_argument(
        '--deployment-target',
        default=os.environ.get('MACOSX_DEPLOYMENT_TARGET', DEFAULT_DEPLOYMENT_TARGET),
        help='Minimum macOS version (default: %(default)s; honors MACOSX_DEPLOYMENT_TARGET)',
    )
    options = parser.parse_args()
    try:
        package(ROOT, options.target, options.deployment_target)
    except (ValueError, OSError, subprocess.CalledProcessError) as error:
        details = error.stderr if isinstance(error, subprocess.CalledProcessError) else str(error)
        print(f'macOS packaging failed: {error}\n{details}', file=sys.stderr)
        return 1
    return 0


if __name__ == '__main__':
    sys.exit(main())
