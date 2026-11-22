#!/usr/bin/env python3
"""Fetch verified official ORT packages; bypass zg-engine's unavailable GitHub pin.

Print shell assignments with --shell, or write GitHub Actions environment with
--github-env. No profile/system installation. Windows retains DirectML support.
"""
import argparse
import hashlib
import os
from pathlib import Path
import shlex
import subprocess
import tempfile
import zipfile

BASE = "https://api.nuget.org/v3-flatcontainer"
PACKAGES = {
    "cpu": ("microsoft.ml.onnxruntime", "1.21.0", "359b4782cfa2e288e6c4bbb9da1c231b085fb8e0d149faf8c3c843f54576559b"),
    "directml": ("microsoft.ml.onnxruntime.directml", "1.21.0", "12bed6e1e870f30e0150e039ebb6853da32eba381eda1c373faeeb17168785e6"),
    "dml": ("microsoft.ai.directml", "1.15.4", "4e7cb7ddce8cf837a7a75dc029209b520ca0101470fcdf275c1f49736a3615b9"),
}
TARGETS = {
    "x86_64-unknown-linux-gnu": ("linux-x64", "libonnxruntime.so.1"),
    "aarch64-unknown-linux-gnu": ("linux-arm64", "libonnxruntime.so.1"),
    "aarch64-apple-darwin": ("osx-arm64", "libonnxruntime.1.dylib"),
    "x86_64-apple-darwin": ("osx-x64", "libonnxruntime.1.dylib"),
    "x86_64-pc-windows-msvc": ("win-x64", "onnxruntime.dll"),
}


def digest(path):
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def package(cache, key):
    name, version, expected = PACKAGES[key]
    archive = cache / f"{name}.{version}.nupkg"
    if not archive.is_file() or digest(archive) != expected:
        cache.mkdir(parents=True, exist_ok=True)
        with tempfile.TemporaryDirectory(dir=cache) as temp:
            partial = Path(temp) / "download"
            subprocess.run(["curl", "-fL", "--retry", "3", "--connect-timeout", "20",
                            "--max-time", "600", "-o", str(partial),
                            f"{BASE}/{name}/{version}/{name}.{version}.nupkg"], check=True)
            if digest(partial) != expected:
                raise RuntimeError(f"SHA-256 mismatch: {name} {version}")
            partial.replace(archive)
    return archive


def prepare(target, cache, destination):
    rid, library = TARGETS[target]
    destination.mkdir(parents=True, exist_ok=True)
    windows = target.endswith("windows-msvc")
    key = "directml" if windows else "cpu"
    source_lib = "onnxruntime.dll" if windows else (
        "libonnxruntime.dylib" if "apple" in target else "libonnxruntime.so")
    with zipfile.ZipFile(package(cache, key)) as archive:
        files = {
            f"runtimes/{rid}/native/{source_lib}": library,
            "LICENSE": "licenses/onnxruntime-LICENSE",
            "ThirdPartyNotices.txt": "licenses/onnxruntime-ThirdPartyNotices.txt",
        }
        for source, name in files.items():
            path = destination / name
            path.parent.mkdir(parents=True, exist_ok=True)
            path.write_bytes(archive.read(source))
    if windows:
        with zipfile.ZipFile(package(cache, "dml")) as archive:
            for source, name in {
                "bin/x64-win/DirectML.dll": "DirectML.dll",
                "LICENSE.txt": "licenses/directml-LICENSE",
                "LICENSE-CODE.txt": "licenses/directml-LICENSE-CODE",
                "ThirdPartyNotices.txt": "licenses/directml-ThirdPartyNotices.txt",
            }.items():
                (destination / name).write_bytes(archive.read(source))
    return destination / library


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--target", required=True, choices=TARGETS)
    parser.add_argument("--cache-dir", required=True, type=Path)
    parser.add_argument("--destination", required=True, type=Path)
    parser.add_argument("--github-env", action="store_true")
    parser.add_argument("--shell", action="store_true")
    args = parser.parse_args()
    library = prepare(args.target, args.cache_dir.resolve(), args.destination.resolve())
    env = {"ORT_DYLIB_PATH": str(library), "OINO_ORT_DIR": str(library.parent)}
    if args.github_env:
        with open(os.environ["GITHUB_ENV"], "a", encoding="utf-8") as stream:
            for key, value in env.items():
                stream.write(f"{key}={value}\n")
    if args.shell:
        for key, value in env.items():
            print(f"export {key}={shlex.quote(value)}")


if __name__ == "__main__":
    main()
