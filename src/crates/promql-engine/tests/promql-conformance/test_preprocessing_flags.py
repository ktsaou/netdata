"""Falsify missing, malformed or misplaced invariant-boundary projections."""
import unittest

from preprocessing_flags import same_boundaries


class PreprocessingFlagTests(unittest.TestCase):
    def test_boundary_order_does_not_change_which_nodes_are_wrapped(self):
        self.assertTrue(same_boundaries({"boundaries": [[1, 4], [6, 9]]}, {"boundaries": [[6, 9], [1, 4]]}))
        self.assertTrue(same_boundaries({"boundaries": []}, {"boundaries": []}))

    def test_missing_extra_moved_and_malformed_boundaries_fail(self):
        source = {"boundaries": [[5, 29]]}
        for target in [{}, {"boundaries": None}, {"boundaries": []},
                       {"boundaries": [[5, 29], [5, 29]]}, {"boundaries": [[0, 30]]},
                       {"boundaries": [[5.0, 29]]}, {"boundaries": [[True, 29]]},
                       {"boundaries": [[5, 29, 30]]}, {"boundaries": ["5,29"]}]:
            with self.subTest(target=target):
                self.assertFalse(same_boundaries(source, target))
        self.assertFalse(same_boundaries({"boundaries": []}, {}))

    def test_errors_keep_their_phase_and_exact_payload(self):
        source = {"error": {"phase": "preprocess", "type": "error", "message": "literal"}}
        self.assertTrue(same_boundaries(source, source))
        self.assertFalse(same_boundaries(source, {"boundaries": []}))
        self.assertFalse(same_boundaries(source, {"error": {"phase": "parse", "type": "error", "message": "literal"}}))
