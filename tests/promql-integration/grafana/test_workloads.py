#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check catalogue binding and interpolation before making live requests."""

import json
import unittest

from measure import interpolate, workloads
from workloads import HERE, build, catalogue


class WorkloadTests(unittest.TestCase):
    def test_every_target_resolves(self):
        cases = workloads([])
        self.assertEqual(len(cases), 136)
        self.assertEqual(len({case['id'] for case, _ in cases}), len(cases))
        for _, target in cases:
            expression = interpolate(target['expr'], [], 30, 900)
            self.assertNotIn('${', expression)
            self.assertNotIn('rate(app_', expression)

    def test_host_regex_and_promql_string_escaping(self):
        expression = interpolate('m{host=~"${host}"}', ['a.example', 'node"x'], 30, 900)
        raw = expression[len('m{host=~'): -1]
        self.assertEqual(json.loads(raw), 'a\\.example|node"x')

    def test_unknown_variable_is_an_error(self):
        with self.assertRaises(ValueError):
            interpolate('m{x="${missing}"}', [], 30, 900)

    def test_generated_files_and_catalogue_match(self):
        generated = build()
        for name, value in generated.items():
            self.assertEqual(json.loads((HERE / name).read_text()), value)
        all_dashboards = {p.name: json.loads(p.read_text()) for p in HERE.glob('*.json')
                          if p.name != 'workload-catalogue.json'}
        self.assertEqual(json.loads((HERE / 'workload-catalogue.json').read_text()), catalogue(all_dashboards))


if __name__ == '__main__':
    unittest.main()
