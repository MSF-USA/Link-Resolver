"""Check packaging/stamp-version.py against copies of the real Cargo files."""

import importlib.util
import pathlib
import shutil
import subprocess
import sys
import tempfile
import unittest

ROOT = pathlib.Path(__file__).resolve().parents[1]
SCRIPT = ROOT / 'packaging' / 'stamp-version.py'


def load_stamper():
    """Import the script as a module despite the hyphen in its file name."""
    spec = importlib.util.spec_from_file_location('stamp_version', SCRIPT)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


stamper = load_stamper()


class VersionParsingTests(unittest.TestCase):
    """The tag-to-crate-version mapping the release workflow relies on."""

    def test_tags_and_plain_versions(self):
        """v1.DATE maps to 1.DATE.0, v1.DATE.N keeps N, plain versions pass through."""
        self.assertEqual(stamper.crate_version('v1.20261106'), '1.20261106.0')
        self.assertEqual(stamper.crate_version('1.20261106'), '1.20261106.0')
        self.assertEqual(stamper.crate_version('v1.20261106.2'), '1.20261106.2')
        self.assertEqual(stamper.crate_version('1.20261106.7'), '1.20261106.7')
        self.assertEqual(stamper.crate_version('0.1.0'), '0.1.0')

    def test_rejects_other_shapes(self):
        """Anything that is not the scheme or a three-part version is an error."""
        for bad in ('v2.20261106', 'v1.2026110', 'v1.20261106.', '1.2', 'latest', ''):
            with self.subTest(bad=bad):
                with self.assertRaises(ValueError):
                    stamper.crate_version(bad)

    def test_release_suffix_limit_is_checked_before_platform_builds(self):
        """The parser used by the version job accepts .99 and rejects .100 in every form."""
        for prefix in ('v1.20261106', '1.20261106'):
            with self.subTest(prefix=prefix):
                self.assertEqual(stamper.crate_version(prefix + '.99'), '1.20261106.99')
                with self.assertRaisesRegex(ValueError, 'suffix above 99'):
                    stamper.crate_version(prefix + '.100')


class StampingTests(unittest.TestCase):
    """Stamping edits exactly the crate's own version in both files and nothing else."""

    def setUp(self):
        """Copy the repository's Cargo.toml and Cargo.lock into a scratch directory."""
        self.scratch = pathlib.Path(tempfile.mkdtemp(prefix='stamp-version-'))
        self.addCleanup(shutil.rmtree, self.scratch, ignore_errors=True)
        for name in ('Cargo.toml', 'Cargo.lock'):
            shutil.copy(ROOT / name, self.scratch / name)
        self.original_lock = (self.scratch / 'Cargo.lock').read_text(encoding='utf-8')

    def test_stamps_manifest_and_matching_lock_entry_only(self):
        """Both files say the new version; every other lock line is unchanged."""
        stamper.stamp('1.20261106.2', self.scratch)
        manifest = (self.scratch / 'Cargo.toml').read_text(encoding='utf-8')
        self.assertIn('\nversion = "1.20261106.2"\n', manifest)
        self.assertEqual(manifest.count('version = "1.20261106.2"'), 1)
        lock = (self.scratch / 'Cargo.lock').read_text(encoding='utf-8')
        self.assertIn(
            '[[package]]\nname = "link-resolver"\nversion = "1.20261106.2"\n', lock,
        )
        before = self.original_lock.splitlines()
        after = lock.splitlines()
        self.assertEqual(len(before), len(after))
        changed = [(a, b) for a, b in zip(before, after) if a != b]
        self.assertEqual(len(changed), 1, changed)
        self.assertTrue(changed[0][0].startswith('version = "'))
        self.assertEqual(changed[0][1], 'version = "1.20261106.2"')

    def test_lock_stays_valid_for_cargo(self):
        """Cargo accepts the stamped lockfile as fully locked (no resolution changes)."""
        stamper.stamp('1.20261106.0', self.scratch)
        # A manifest needs a source file to be loadable; the real sources are not copied.
        (self.scratch / 'src').mkdir()
        (self.scratch / 'src' / 'lib.rs').write_text('', encoding='utf-8')
        (self.scratch / 'src' / 'main.rs').write_text('fn main() {}', encoding='utf-8')
        (self.scratch / 'src' / 'bin').mkdir()
        (self.scratch / 'src' / 'bin' / 'link-resolver-cli.rs').write_text(
            'fn main() {}', encoding='utf-8',
        )
        if shutil.which('cargo') is None:
            self.skipTest('cargo is not installed')
        result = subprocess.run(
            ['cargo', 'metadata', '--locked', '--offline', '--format-version', '1',
             '--no-deps'],
            cwd=self.scratch, capture_output=True, text=True, timeout=120,
        )
        if 'offline' in result.stderr and result.returncode != 0:
            self.skipTest('no local registry index to check the lockfile against')
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('"version":"1.20261106.0"', result.stdout.replace(' ', ''))

    def test_missing_entry_is_an_error_and_leaves_files_alone(self):
        """A lockfile without the crate's entry fails before anything is written."""
        (self.scratch / 'Cargo.lock').write_text('version = 4\n', encoding='utf-8')
        with self.assertRaises(ValueError):
            stamper.stamp('1.20261106.0', self.scratch)
        manifest = (self.scratch / 'Cargo.toml').read_text(encoding='utf-8')
        self.assertNotIn('1.20261106.0', manifest)

    def test_out_of_range_release_suffix_fails_without_stamping_files(self):
        """CLI validation rejects .100 before changing the manifest or lockfile."""
        before = {name: (self.scratch / name).read_bytes() for name in ('Cargo.toml', 'Cargo.lock')}
        script_copy = self.scratch / 'packaging' / 'stamp-version.py'
        script_copy.parent.mkdir()
        shutil.copy(SCRIPT, script_copy)
        result = subprocess.run(
            [sys.executable, str(script_copy), 'v1.20261106.100'],
            cwd=self.scratch, capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(result.returncode, 1)
        self.assertIn('suffix above 99', result.stderr)
        for name, contents in before.items():
            self.assertEqual((self.scratch / name).read_bytes(), contents)

    def test_command_line_round_trip(self):
        """The script as the workflow calls it exits 0 and reports both files."""
        env_root = self.scratch
        # Run the real script against the copies by pointing a temporary copy's ROOT at them.
        script_copy = self.scratch / 'packaging' / 'stamp-version.py'
        script_copy.parent.mkdir()
        shutil.copy(SCRIPT, script_copy)
        result = subprocess.run(
            [sys.executable, str(script_copy), 'v1.20261106.3'],
            cwd=env_root, capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn('1.20261106.3', result.stdout)
        self.assertIn('version = "1.20261106.3"', (env_root / 'Cargo.toml').read_text())
        bad = subprocess.run(
            [sys.executable, str(script_copy), 'v9'], cwd=env_root,
            capture_output=True, text=True, timeout=60,
        )
        self.assertEqual(bad.returncode, 1)
        self.assertIn('neither', bad.stderr)
