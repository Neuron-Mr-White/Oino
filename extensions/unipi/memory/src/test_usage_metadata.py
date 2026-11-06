"""Pure adapter acceptance, no installed palace or user database required."""
import importlib.util
import json
import pathlib
import unittest

spec = importlib.util.spec_from_file_location("usage_metadata", pathlib.Path(__file__).with_name("usage_metadata.py"))
usage = importlib.util.module_from_spec(spec)
spec.loader.exec_module(usage)


class MetadataMigration(unittest.TestCase):
    def test_sidecar_import_preserves_metadata_and_is_idempotent(self):
        original = {"source_file": "/p/a.md", "custom": "keep", "last_used_at": 50,
                    "use_count": 2, "usage_sources": '["old"]'}
        incoming = {"lastUsed": 100, "useCount": 3, "sources": ["new"], "mergedFrom": ["p/old"]}
        merged = usage.merge_usage(original, incoming)
        self.assertEqual(merged, usage.merge_usage(merged, incoming))
        self.assertEqual(merged["last_used_at"], 100)
        self.assertEqual(merged["use_count"], 3)
        self.assertEqual(json.loads(merged["usage_sources"]), ["old", "new"])
        self.assertEqual(json.loads(merged["usage_merged_from"]), ["p/old"])
        self.assertEqual(merged["custom"], "keep")
        self.assertEqual(merged["source_file"], "/p/a.md")
        self.assertEqual(original["use_count"], 2)


if __name__ == "__main__":
    unittest.main()
