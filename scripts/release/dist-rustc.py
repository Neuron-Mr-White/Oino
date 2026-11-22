#!/usr/bin/env python3
"""xtask-only rustc wrapper: keep crate disambiguators independent of Cargo paths.

Cargo hashes absolute local package IDs AND absolute remap flags into -C metadata.
Passing another -C metadata adds a salt rather than replacing the Cargo salt.
Replace it here, preserving crate/version/features/target identity. This wrapper
is only for isolated extension-dist builds, never ordinary workspace builds.
"""
import hashlib
import json
import os
import sys

compiler, *args = sys.argv[1:]
identity = [os.environ.get(key, "") for key in ("CARGO_PKG_NAME", "CARGO_PKG_VERSION")]
for index, arg in enumerate(args):
    if arg in ("--crate-name", "--crate-type", "--target", "--cfg") and index + 1 < len(args):
        identity.extend((arg, args[index + 1]))
salt = hashlib.sha256(json.dumps(identity).encode()).hexdigest()[:16]
output = []
index = 0
while index < len(args):
    arg = args[index]
    if arg == "-C" and index + 1 < len(args) and args[index + 1].startswith("metadata="):
        output.extend(("-C", f"metadata={salt}"))
        index += 2
        continue
    if arg.startswith("-Cmetadata="):
        index += 1
        continue
    output.append(arg)
    index += 1
os.execv(compiler, [compiler, *output])
