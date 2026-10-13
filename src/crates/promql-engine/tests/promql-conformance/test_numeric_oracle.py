"""Check the Python comparator against the actual pinned Go almost.Equal implementation."""
import json
from pathlib import Path
import random
import struct
import subprocess
import tempfile
import unittest

from compare import compare, same_number


class NumericReferenceTests(unittest.TestCase):
    def test_binary64_comparator_against_pinned_upstream(self):
        scratch = Path(__file__).resolve().parent / ".tmp"
        oracle = scratch / "oracle"
        self.assertTrue(oracle.is_file(), "build the pinned oracle before running conformance tests")
        rng = random.Random(315)
        special = [0, 1, (1 << 63), 0x0010000000000000, 0x7FF0000000000000,
                   0xFFF0000000000000, 0x7FF0000000000002, 0x7FF8000000000001, 0x7FF8000000000002]
        pairs = [(a, b) for a in special for b in special]
        for _ in range(2048):
            a = rng.getrandbits(64)
            pairs.extend([(a, rng.getrandbits(64)), (a, (a + 1) & ((1 << 64) - 1))])
        encoded = [[f"{a:016x}", f"{b:016x}"] for a, b in pairs]
        folder = Path(tempfile.mkdtemp(prefix="numeric-", dir=scratch))
        path = folder / "pairs.json"
        path.write_text(json.dumps(encoded))
        expected = json.loads(subprocess.check_output([oracle, "--numbers", path]))
        actual = [same_number(a, b) for a, b in encoded]
        self.assertEqual(actual, expected)

    def test_bucket_layout_equivalence_against_upstream_compact(self):
        scratch = Path(__file__).resolve().parent / ".tmp"
        oracle = scratch / "oracle"
        self.assertTrue(oracle.is_file(), "build the pinned oracle before running conformance tests")
        raw = lambda value: struct.pack(">d", value).hex()
        rng = random.Random(5241)
        histograms = []
        for _ in range(128):
            histogram = {"reset_hint": 3, "schema": 0, "count": raw(20), "sum": raw(20),
                         "zero_count": raw(0), "zero_threshold": raw(0), "custom_values": None}
            for side in ["positive", "negative"]:
                spans, buckets = [], []
                for index in range(rng.randrange(1, 5)):
                    length = rng.randrange(0, 7)
                    spans.append({"Offset": rng.randrange(-3 if index == 0 else 0, 4), "Length": length})
                    buckets.extend(raw(rng.choice([0, -0.0, 0, 1, 2, 3])) for _ in range(length))
                histogram[side + "_spans"], histogram[side + "_buckets"] = spans, buckets
            histograms.append(histogram)
        folder = Path(tempfile.mkdtemp(prefix="histograms-", dir=scratch))
        path = folder / "histograms.json"
        path.write_text(json.dumps(histograms))
        compacted = json.loads(subprocess.check_output([oracle, "--compact-histograms", path]))
        def document(histogram):
            return {"schema_version": 1, "results": [{"id": "histogram", "kind": "vector", "warnings": [], "infos": [],
                "rows": [{"labels": [], "points": [{"time_ms": 0, "kind": "float_histogram", "float_histogram": histogram}]}]}]}
        for index, (original, compact) in enumerate(zip(histograms, compacted)):
            with self.subTest(index=index):
                self.assertTrue(compare(document(original), document(compact), [{"id": "histogram"}])["semantic_pass"])


if __name__ == "__main__":
    unittest.main()
