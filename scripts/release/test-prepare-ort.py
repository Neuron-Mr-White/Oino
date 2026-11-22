#!/usr/bin/env python3
"""Offline unit tests for verified runtime acquisition and extraction."""
import hashlib
import importlib.util
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch
import zipfile

spec = importlib.util.spec_from_file_location("prepare_ort", Path(__file__).with_name("prepare-ort.py"))
ort = importlib.util.module_from_spec(spec)
spec.loader.exec_module(ort)


class PrepareTests(unittest.TestCase):
    def test_all_cpu_targets(self):
        with tempfile.TemporaryDirectory() as temp:
            cache = Path(temp)
            archive = cache / "fixture.1.nupkg"
            with zipfile.ZipFile(archive, "w") as z:
                z.writestr("LICENSE", "license")
                z.writestr("ThirdPartyNotices.txt", "notices")
                for rid, _ in ort.TARGETS.values():
                    source = "libonnxruntime.dylib" if rid.startswith("osx") else "libonnxruntime.so"
                    z.writestr(f"runtimes/{rid}/native/{source}", rid)
            pin = ("fixture", "1", ort.digest(archive))
            with patch.dict(ort.PACKAGES, {"cpu": pin}):
                for target, (rid, name) in ort.TARGETS.items():
                    if "windows" in target:
                        continue
                    output = ort.prepare(target, cache, cache / target)
                    self.assertEqual(output.name, name)
                    self.assertEqual(output.read_text(), rid)
                    self.assertTrue((output.parent / "licenses/onnxruntime-LICENSE").is_file())

    def test_corrupt_download_is_never_published(self):
        with tempfile.TemporaryDirectory() as temp:
            cache = Path(temp)
            pin = ("fixture", "1", hashlib.sha256(b"good").hexdigest())
            def download(args, **_):
                Path(args[args.index("-o") + 1]).write_bytes(b"corrupt")
            with patch.dict(ort.PACKAGES, {"cpu": pin}), patch.object(ort.subprocess, "run", download):
                with self.assertRaisesRegex(RuntimeError, "SHA-256 mismatch"):
                    ort.package(cache, "cpu")
            self.assertFalse((cache / "fixture.1.nupkg").exists())

    def test_verified_cache_needs_no_network(self):
        with tempfile.TemporaryDirectory() as temp:
            cache = Path(temp)
            archive = cache / "fixture.1.nupkg"
            archive.write_bytes(b"verified")
            with patch.dict(ort.PACKAGES, {"cpu": ("fixture", "1", ort.digest(archive))}), patch.object(ort.subprocess, "run") as run:
                self.assertEqual(ort.package(cache, "cpu"), archive)
                run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
