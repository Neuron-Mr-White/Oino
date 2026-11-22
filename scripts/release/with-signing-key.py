#!/usr/bin/env python3
"""Run a local release command with the documented Tauri signing environment.

Never prints credentials. CI should inject the same environment directly.
"""
import os
from pathlib import Path
import re
import sys


def main():
    if len(sys.argv) < 2:
        sys.exit('usage: with-signing-key.py COMMAND [ARG ...]')
    env = os.environ.copy()
    android = sys.argv[1] == '--android'
    command = sys.argv[2:] if android else sys.argv[1:]
    if not command:
        sys.exit('signing wrapper requires a command')
    if android:
        labels = {'file': 'UNIPI_ANDROID_KEYSTORE', 'alias': 'UNIPI_ANDROID_KEY_ALIAS',
                  'store password': 'UNIPI_ANDROID_KEYSTORE_PASSWORD',
                  'key password': 'UNIPI_ANDROID_KEY_PASSWORD'}
        if any(not env.get(name) for name in labels.values()):
            lines = (Path.home() / '.oino-release-keys/README.txt').read_text().splitlines()
            for line in lines:
                if ':' not in line:
                    continue
                label, value = line.split(':', 1)
                if label.strip() in labels:
                    name = labels[label.strip()]
                    if not env.get(name):
                        env[name] = os.path.expanduser(value.strip().strip('`'))
        if any(not env.get(name) for name in labels.values()):
            sys.exit('Android signing fields missing')
        os.execvpe(command[0], command, env)
    if not env.get('TAURI_SIGNING_PRIVATE_KEY'):
        keys = Path.home() / '.oino-release-keys'
        key = keys / 'tauri-updater.key'
        if not key.is_file():
            sys.exit('Tauri signing key missing; see docs/release-feeds.md')
        env['TAURI_SIGNING_PRIVATE_KEY'] = str(key)
        if 'TAURI_SIGNING_PRIVATE_KEY_PASSWORD' not in env:
            section = (keys / 'README.txt').read_text().split('== Tauri updater (minisign) ==')[1].split('== GitHub')[0]
            match = re.search(r'^password:\s+(\S+)', section, re.M)
            if not match:
                sys.exit('Tauri signing password missing in local key instructions')
            env['TAURI_SIGNING_PRIVATE_KEY_PASSWORD'] = match[1]
    os.execvpe(command[0], command, env)


if __name__ == '__main__':
    main()
