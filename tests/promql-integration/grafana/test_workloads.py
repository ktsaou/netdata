#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Check catalogue binding and interpolation before making live requests."""

import json
import os
import struct
import subprocess
import sys
import unittest
from pathlib import Path

from measure import interpolate, workloads
from workloads import EXISTING_DASHBOARDS, HERE, build, catalogue


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

    @unittest.skipUnless(os.environ.get('PROMQL_QUERY_FIXTURE'), 'set PROMQL_QUERY_FIXTURE for numerical integration')
    def test_packet_ratio_does_not_count_multicast_twice(self):
        helpers = HERE.parents[2] / 'src/crates/promql-engine/tests/promql-conformance'
        sys.path.insert(0, str(helpers))
        from info_cases import series
        from query import dataset_hex, request_hex, result_base64
        _, target = next(row for row in workloads([]) if row[0]['title'] == 'Network drops per observed packet')
        rows = []
        for name, dimension, value in [('net_packets', 'received', 100), ('net_packets', 'sent', 50),
                                       ('net_packets', 'multicast', 25), ('net_drops', 'inbound', 15)]:
            rows.append(series({'__name__': name, 'netdata_host_name': 'fixture', 'netdata_dimension_id': dimension},
                               [(t, value * t / 1000, 'float') for t in [0, 15000, 30000, 45000, 60000]]))
        query = {'id': 'packet-ratio', 'query': interpolate(target['expr'], [], 30, 900), 'profile': 'server-default',
                 'mode': 'instant', 'time_ms': 60000, 'start_ms': 60000, 'end_ms': 60000,
                 'step_ms': 30000, 'lookback_ms': 300000, 'subquery_step_ms': 60000}
        messages = [{'op': 'dataset', 'dataset_id': 'm', 'dataset': dataset_hex({'schema_version': 1, 'series': rows}),
                     'page_size': 17}, request_hex(query, 'm')]
        binary = Path(os.environ['PROMQL_QUERY_FIXTURE']).resolve(strict=True)
        result = subprocess.run([str(binary)], input=('\n'.join(map(json.dumps, messages)) + '\n').encode(),
                                capture_output=True, check=True)
        packets = [json.loads(line) for line in result.stdout.splitlines()]
        value = result_base64(next(packet['result'] for packet in packets if packet.get('id') == 'packet-ratio'))
        actual = struct.unpack('>d', bytes.fromhex(value['rows'][0]['points'][0]['float']))[0]
        self.assertAlmostEqual(actual, 0.1, places=14)

    @unittest.skipUnless(os.environ.get('PROMQL_QUERY_FIXTURE'), 'set PROMQL_QUERY_FIXTURE for numerical integration')
    def test_historical_comparison_preserves_host_matcher(self):
        helpers = HERE.parents[2] / 'src/crates/promql-engine/tests/promql-conformance'
        sys.path.insert(0, str(helpers))
        from info_cases import series
        from query import dataset_hex, request_hex, result_base64
        rows = [series({'__name__': 'system_cpu', 'netdata_host_name': 'fixture', 'netdata_host_id': 'fixture-id',
                        'netdata_dimension_id': 'idle'}, [(0, 80, 'float'), (3600000, 50, 'float')])]
        cases = [row for row in workloads([]) if row[0]['title'] in
                 ('CPU busy change versus one hour ago', 'CPU increase from one hour ago above 20 points')]
        messages = [{'op': 'dataset', 'dataset_id': 'm', 'dataset': dataset_hex({'schema_version': 1, 'series': rows}),
                     'page_size': 17}]
        for case, target in cases:
            query = {'id': case['id'], 'query': interpolate(target['expr'], ['fixture'], 30, 900),
                     'profile': 'server-default', 'mode': 'instant', 'time_ms': 3600000, 'start_ms': 3600000,
                     'end_ms': 3600000, 'step_ms': 30000, 'lookback_ms': 300000, 'subquery_step_ms': 60000}
            messages.append(request_hex(query, 'm'))
        binary = Path(os.environ['PROMQL_QUERY_FIXTURE']).resolve(strict=True)
        result = subprocess.run([str(binary)], input=('\n'.join(map(json.dumps, messages)) + '\n').encode(),
                                capture_output=True, check=True)
        packets = [json.loads(line) for line in result.stdout.splitlines()]
        actual = [result_base64(packet['result']) for packet in packets if 'id' in packet]
        self.assertEqual(len(actual), 2)
        for value in actual:
            self.assertEqual(len(value['rows']), 1, value)
            self.assertEqual(struct.unpack('>d', bytes.fromhex(value['rows'][0]['points'][0]['float']))[0], 30)

    def test_generated_files_and_catalogue_match(self):
        generated = build()
        for name, value in generated.items():
            self.assertEqual(json.loads((HERE / name).read_text()), value)
        all_dashboards = {name: json.loads((HERE / name).read_text()) for name in EXISTING_DASHBOARDS}
        all_dashboards.update(generated)
        self.assertEqual(json.loads((HERE / 'workload-catalogue.json').read_text()), catalogue(all_dashboards))


if __name__ == '__main__':
    unittest.main()
