"""Offline numeric-capture admission checks; no JVM or generated outputs."""
from contextlib import redirect_stderr
import io
from pathlib import Path
import sys
import unittest
from unittest.mock import patch

import numeric


class NumericCaptureTests(unittest.TestCase):
    def test_target_adapter_requires_the_exact_preserved_codec_seam(self):
        source = (numeric.native.ROOT / "scripts/NumericReference.java").read_bytes()
        adapted = numeric.adapt_probe(source)
        self.assertNotIn(b"HOLDER_HELPER_CODEC", adapted)
        self.assertEqual(adapted.count(b'field("world.level.levelgen.DensityFunction", "CODEC")'), 1)
        for changed in (adapted, source + source):
            with self.subTest(length=len(changed)), self.assertRaises(ValueError):
                numeric.adapt_probe(changed)

    def test_target_requests_retain_all_base_cases_and_add_codec_boundaries(self):
        sys.path.insert(0, str(numeric.native.ROOT / "scripts"))
        import numeric_cases

        with patch.object(numeric_cases, "requests", return_value={
            "points": {"blocks": [[0, 0, 0]]}, "cases": [{"id": "base"}],
        }) as base:
            result = numeric.requests()
        base.assert_called_once_with(numeric.native.JAR)
        self.assertEqual(result["cases"][0], {"id": "base"})
        self.assertEqual(len(result["cases"]), 16)
        self.assertEqual(len({row["id"] for row in result["interval_codecs"]}), 18)
        gradient = result["cases"][-1]["function"]["input"]
        self.assertEqual(gradient["type"], "minecraft:y_clamped_gradient")
        self.assertEqual(result["interval_runtime"]["inputs"][-1], "7ff8000000000011")

    def test_existing_output_is_rejected_before_native_execution(self):
        with patch.object(numeric.native, "environment", side_effect=AssertionError("native invocation")), \
                patch.object(sys, "argv", ["numeric.py", "capture", "--output", str(Path(__file__).parent)]), \
                redirect_stderr(io.StringIO()), self.assertRaises(SystemExit) as error:
            numeric.main()
        self.assertEqual(error.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
