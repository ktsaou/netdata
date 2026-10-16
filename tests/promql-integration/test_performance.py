#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check the measurement caller against the actual PromQL HTTP envelope."""

import json
import unittest

from performance import cases, validate


class PerformanceEnvelopeTests(unittest.TestCase):
    def test_statistics_belong_inside_data(self):
        case = cases({"base": 1700002800, "host": "fixture"})[0]
        stats = {"timings": {"evalTotalTime": 0.000104369, "resultSortTime": 0.00000131,
                             "queryPreparationTime": 0.000024469, "innerEvalTime": 0.00005701,
                             "execQueueTime": 0, "execTotalTime": 0.00010651},
                 "samples": {"totalQueryableSamples": 0, "peakSamples": 1}}
        response = {"status": 200, "headers": {}}
        value = {"status": "success", "data": {"resultType": "scalar", "result": [1700002804, "1"], "stats": stats}}
        self.assertEqual(validate(case, response, json.dumps(value).encode()), stats)
        value["stats"] = value["data"].pop("stats")
        with self.assertRaises(RuntimeError):
            validate(case, response, json.dumps(value).encode())


if __name__ == "__main__":
    unittest.main()
