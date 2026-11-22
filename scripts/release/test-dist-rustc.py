#!/usr/bin/env python3
"""Regression tests for the dist-only stable crate disambiguator."""
import os
from pathlib import Path
import runpy
import sys
import unittest
from unittest.mock import patch

WRAPPER = Path(__file__).with_name("dist-rustc.py")


def invocation(metadata, version="1", feature="default", target="wasm32-wasip2"):
    args = [str(WRAPPER), "/toolchain/rustc", "--crate-name", "fixture", "--crate-type", "rlib",
            "--target", target, "--cfg", f'feature="{feature}"', "-C", f"metadata={metadata}",
            "--out-dir", "/arbitrary/output"]
    with patch.object(sys, "argv", args), patch.dict(os.environ, {"CARGO_PKG_NAME": "fixture", "CARGO_PKG_VERSION": version}), patch("os.execv") as execute:
        runpy.run_path(str(WRAPPER))
        return execute.call_args.args[1]


class WrapperTests(unittest.TestCase):
    def test_cargo_path_salt_does_not_change_metadata(self):
        self.assertEqual(invocation("checkout-a"), invocation("checkout-b"))

    def test_distinct_versions_features_and_targets_stay_distinct(self):
        base = invocation("a")
        for kwargs in ({"version": "2"}, {"feature": "provider"}, {"target": "x86_64-unknown-linux-musl"}):
            self.assertNotEqual(base, invocation("a", **kwargs))

    def test_other_arguments_are_preserved(self):
        args = invocation("a")
        self.assertEqual(args[0], "/toolchain/rustc")
        self.assertEqual(args[-2:], ["--out-dir", "/arbitrary/output"])
        self.assertEqual(sum(arg.startswith("metadata=") for arg in args), 1)


if __name__ == "__main__":
    unittest.main()
