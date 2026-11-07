"""Exercise the resolver scripts against a local HTTP redirect server."""

import http.server
import os
import pathlib
import shutil
import subprocess
import sys
import threading
import unittest


ROOT = pathlib.Path(__file__).resolve().parents[1]


def find_bash():
    """Return the path of a Bash that can run scripts, or None when there is none.

    Windows ships ``C:\\Windows\\System32\\bash.exe``, which only launches WSL and
    prints an error when no distribution is installed, so ``shutil.which('bash')`` is
    not enough there. Git for Windows' Bash is tried first, then whatever is on PATH,
    and a candidate counts only if it actually runs a command.
    """
    candidates = []
    if sys.platform == 'win32':
        git = shutil.which('git')
        if git:
            # C:\Program Files\Git\cmd\git.exe -> C:\Program Files\Git\bin\bash.exe
            candidates.append(pathlib.Path(git).resolve().parents[1] / 'bin' / 'bash.exe')
        for base in (os.environ.get('ProgramFiles'), os.environ.get('ProgramFiles(x86)')):
            if base:
                candidates.append(pathlib.Path(base) / 'Git' / 'bin' / 'bash.exe')
    on_path = shutil.which('bash')
    if on_path:
        candidates.append(pathlib.Path(on_path))
    for candidate in candidates:
        if not candidate.is_file():
            continue
        try:
            probe = subprocess.run(
                [str(candidate), '-c', 'echo probe-ok'], capture_output=True,
                text=True, timeout=20,
            )
        except (OSError, subprocess.TimeoutExpired):
            continue
        if probe.returncode == 0 and probe.stdout.strip() == 'probe-ok':
            return str(candidate)
    return None


class RedirectHandler(http.server.BaseHTTPRequestHandler):
    """Serve redirect chains, loops, and final error responses for tests."""

    def do_GET(self):
        """Handle a GET request without a body; no return value is produced."""
        destinations = {
            '/start': '/middle',
            '/middle': '/final?source=redirect',
            '/loop': '/loop',
            '/to-error': '/missing',
        }
        if self.path in destinations:
            self.send_response(302)
            self.send_header('Location', destinations[self.path])
        else:
            self.send_response(404 if self.path == '/missing' else 200)
        self.send_header('Content-Length', '0')
        self.end_headers()

    def log_message(self, format, *args):
        """Suppress server logging during tests; returns None."""


class ResolverTests(unittest.TestCase):
    """Verify output and failure behavior for each available script runtime."""

    @classmethod
    def setUpClass(cls):
        """Start a loopback HTTP server; socket errors propagate to unittest."""
        cls.server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), RedirectHandler)
        cls.thread = threading.Thread(target=cls.server.serve_forever, daemon=True)
        cls.thread.start()
        cls.base_url = f'http://127.0.0.1:{cls.server.server_port}'

    @classmethod
    def tearDownClass(cls):
        """Stop and close the test server; returns None."""
        cls.server.shutdown()
        cls.server.server_close()
        cls.thread.join()

    def check_resolver(self, command):
        """Assert a script's results for redirects and errors; raises on failure."""
        cases = [
            ('/start', '/final?source=redirect'),
            ('/final', '/final'),
            ('/to-error', '/missing'),
        ]
        for path, expected in cases:
            with self.subTest(command=command[0], path=path):
                result = subprocess.run(
                    command + [self.base_url + path], capture_output=True,
                    text=True, timeout=40,
                )
                self.assertEqual(result.returncode, 0, result.stderr)
                self.assertEqual(result.stdout.strip(), self.base_url + expected)

        for args in ([], ['file:///etc/hosts'], [self.base_url + '/loop']):
            with self.subTest(command=command[0], args=args):
                result = subprocess.run(
                    command + args, capture_output=True, text=True, timeout=40,
                )
                self.assertNotEqual(result.returncode, 0)
                self.assertEqual(result.stdout, '')
                self.assertTrue(result.stderr)

    def test_bash(self):
        """Check the Bash resolver when a working Bash exists; otherwise skip."""
        bash = find_bash()
        if bash is None:
            self.skipTest('No working Bash is installed')
        self.check_resolver([bash, str(ROOT / 'resolve-url.sh')])

    def test_powershell(self):
        """Check PowerShell when installed; otherwise report a skipped test."""
        executable = shutil.which('pwsh') or shutil.which('powershell')
        if executable is None:
            self.skipTest('PowerShell is not installed')
        self.check_resolver([
            executable, '-NoProfile', '-NonInteractive', '-File',
            str(ROOT / 'resolve-url.ps1'),
        ])
