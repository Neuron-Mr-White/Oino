#!/usr/bin/env python3
"""Credential-free regression checks for the local signing wrapper."""
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

WRAPPER = Path(__file__).with_name('with-signing-key.py')


class SigningEnvironment(unittest.TestCase):
    def run_wrapper(self, home, code, extra=None):
        env = {k: v for k, v in os.environ.items() if not k.startswith('TAURI_SIGNING_')}
        env['HOME'] = str(home)
        env.update(extra or {})
        return subprocess.run([sys.executable, str(WRAPPER), sys.executable, '-c', code],
                              env=env, capture_output=True, text=True)

    def test_local_key_loaded_without_output(self):
        with tempfile.TemporaryDirectory() as home:
            keys = Path(home) / '.oino-release-keys'
            keys.mkdir()
            (keys / 'tauri-updater.key').write_text('fake encrypted key')
            (keys / 'README.txt').write_text('== Tauri updater (minisign) ==\npassword: fixture-password\n== GitHub secrets ==\n')
            result = self.run_wrapper(home, 'import os; assert os.environ["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] == "fixture-password"; assert os.path.isfile(os.environ["TAURI_SIGNING_PRIVATE_KEY"])')
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout + result.stderr, '')

    def test_ci_environment_needs_no_local_key(self):
        with tempfile.TemporaryDirectory() as home:
            result = self.run_wrapper(home, 'import os; assert os.environ["TAURI_SIGNING_PRIVATE_KEY"] == "ci-key"; assert os.environ["TAURI_SIGNING_PRIVATE_KEY_PASSWORD"] == "ci-password"',
                                      {'TAURI_SIGNING_PRIVATE_KEY': 'ci-key', 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD': 'ci-password'})
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_android_local_fields_are_private(self):
        with tempfile.TemporaryDirectory() as home:
            keys = Path(home) / '.oino-release-keys'
            keys.mkdir()
            (keys / 'README.txt').write_text('file: ~/fixture.jks\nalias: fixture\nstore password: fixture-store\nkey password: fixture-key\n')
            env = {k: v for k, v in os.environ.items() if not k.startswith('UNIPI_ANDROID_KEY')}
            env['HOME'] = home
            code = 'import os; assert os.environ["UNIPI_ANDROID_KEY_ALIAS"] == "fixture"; assert os.environ["UNIPI_ANDROID_KEYSTORE_PASSWORD"] == "fixture-store"; assert os.environ["UNIPI_ANDROID_KEY_PASSWORD"] == "fixture-key"; assert os.environ["UNIPI_ANDROID_KEYSTORE"] == os.path.expanduser("~/fixture.jks")'
            result = subprocess.run([sys.executable, str(WRAPPER), '--android', sys.executable, '-c', code], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout + result.stderr, '')

    def test_android_ci_needs_no_local_files(self):
        with tempfile.TemporaryDirectory() as home:
            env = dict(os.environ, HOME=home, UNIPI_ANDROID_KEYSTORE='fixture.jks',
                       UNIPI_ANDROID_KEY_ALIAS='fixture', UNIPI_ANDROID_KEYSTORE_PASSWORD='store',
                       UNIPI_ANDROID_KEY_PASSWORD='key')
            result = subprocess.run([sys.executable, str(WRAPPER), '--android', sys.executable, '-c', 'raise SystemExit(7)'], env=env, capture_output=True, text=True)
            self.assertEqual(result.returncode, 7)
            self.assertEqual(result.stdout + result.stderr, '')

    def test_missing_key_fails_closed(self):
        with tempfile.TemporaryDirectory() as home:
            result = self.run_wrapper(home, 'raise AssertionError("must not run")')
            self.assertNotEqual(result.returncode, 0)
            self.assertIn('signing key missing', result.stderr)

    def test_child_exit_status_preserved(self):
        with tempfile.TemporaryDirectory() as home:
            result = self.run_wrapper(home, 'raise SystemExit(7)', {'TAURI_SIGNING_PRIVATE_KEY': 'ci-key'})
            self.assertEqual(result.returncode, 7)


if __name__ == '__main__':
    unittest.main()
