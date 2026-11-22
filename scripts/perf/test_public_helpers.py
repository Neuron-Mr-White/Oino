"""Offline regression checks for the public performance scripts' dependencies."""
import importlib.util
import json
from pathlib import Path
import subprocess
import sys
import unittest

HERE = Path(__file__).resolve().parent


class PublicHelpers(unittest.TestCase):
    def test_fake_model_imports_without_private_qa_tree(self):
        spec = importlib.util.spec_from_file_location("perf_fakellm", HERE / "fakellm.py")
        module = importlib.util.module_from_spec(spec)
        sys.path.insert(0, str(HERE))
        try:
            spec.loader.exec_module(module)
        finally:
            sys.path.pop(0)
        self.assertEqual(list(module.chunks("abc", 2)), ["ab", "c"])
        self.assertEqual(module.text_of({"content": [{"text": "hello"}]}), "hello")
        self.assertEqual(module.delta(content="hello")["choices"][0]["delta"], {"content": "hello"})
        json.dumps(module.delta(tool_calls=[{"index": 0}], finish="tool_calls"))

    def test_app_rig_paths_are_public_and_syntax_valid(self):
        source = (HERE / "app.py").read_text()
        self.assertNotIn("scripts/qa/", source)
        subprocess.run(["bash", "-n", str(HERE / "rig.sh")], check=True)
        subprocess.run(["node", "--check", str(HERE / "rig.mjs")], check=True)
        rig = (HERE / "rig.sh").read_text()
        self.assertNotIn('cp "$HOME/.oino/', rig)
        for script in ["fakellm.py", "app.py"]:
            subprocess.run([sys.executable, str(HERE / script), "--help"], check=True, capture_output=True)


if __name__ == "__main__":
    unittest.main()
