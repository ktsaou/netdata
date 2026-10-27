#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Measure catalogue expressions through a supplied PromQL endpoint; never mutate the server."""

import argparse
import gzip
import hashlib
import json
import re
import time
import urllib.error
import urllib.parse
import urllib.request
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

HERE = Path(__file__).resolve().parent


def interpolate(expression, hosts, step, duration):
    # Match Prometheus multi-select regex escaping, then escape the PromQL string.
    host = '.+' if not hosts else '|'.join(re.escape(value) for value in hosts)
    host = json.dumps(host)[1:-1]
    substitutions = {'${host}': host, '${application}': '.+', '${device}': '.+', '${disk}': '.+',
                     '${interface}': '.+', '${mount}': '.+',
                     '$__rate_interval': f'{max(step + 1, 4)}s', '$__interval': f'{step}s',
                     '$__range': f'{duration}s'}
    for key, value in substitutions.items():
        expression = expression.replace(key, value)
    if re.search(r'\$\{|\$__', expression):
        raise ValueError('unresolved dashboard variable')
    return expression


def workloads(selected):
    catalogue = json.loads((HERE / 'workload-catalogue.json').read_text())
    dashboards = {}
    result = []
    for case in catalogue['workloads']:
        if selected and not any(value in case['id'] for value in selected):
            continue
        dashboard = dashboards.setdefault(case['file'], json.loads((HERE / case['file']).read_text()))
        panel = next(p for p in dashboard['panels'] if p['id'] == case['panel'])
        target = next(t for t in panel['targets'] if t['refId'] == case['target'])
        result.append((case, target))
    return result


def result_fingerprints(data):
    rows = data.get('result', [])
    ordered = json.dumps(rows, sort_keys=True).encode()
    # Preserve complete rows and duplicates while separating emission order from content.
    unordered = (json.dumps(sorted(json.dumps(row, sort_keys=True) for row in rows)).encode()
                 if data.get('resultType') in ('vector', 'matrix') else ordered)
    return {'result_type': data.get('resultType'),
            'result_sha256': hashlib.sha256(ordered).hexdigest(),
            'result_unordered_sha256': hashlib.sha256(unordered).hexdigest()}


def request(base, expression, instant, start, end, step, deadline):
    endpoint = '/api/v1/query' if instant else '/api/v1/query_range'
    params = {'query': expression, 'timeout': f'{deadline}s', 'stats': 'all', 'options': 'absolute'}
    params.update({'time': end} if instant else {'start': start, 'end': end, 'step': step})
    req = urllib.request.Request(base.rstrip('/') + endpoint, urllib.parse.urlencode(params).encode(),
                                 headers={'Content-Type': 'application/x-www-form-urlencoded', 'Accept-Encoding': 'gzip'})
    before = time.monotonic()
    try:
        try:
            response = urllib.request.urlopen(req, timeout=deadline + 10)
        except urllib.error.HTTPError as error:
            response = error
        with response:
            raw = response.read()
            size = len(raw)
            if response.headers.get('Content-Encoding') == 'gzip':
                raw = gzip.decompress(raw)
            value = json.loads(raw)
            data = value.get('data', {})
            rows = data.get('result', [])
            if data.get('resultType') in ('scalar', 'string'):
                series, points = 1, 1
            else:
                series = len(rows)
                points = sum(len(row.get('values', [])) + len(row.get('histograms', [])) +
                             int('value' in row) + int('histogram' in row) for row in rows)
            return {'http_status': response.status, 'status': value.get('status'),
                    'error_type': value.get('errorType'), 'seconds': time.monotonic() - before,
                    'wire_bytes': size, 'series': series, 'points': points,
                    **result_fingerprints(data),
                    'stats': data.get('stats', value.get('stats')), 'warning_count': len(value.get('warnings', [])),
                    'info_count': len(value.get('infos', [])),
                    'outcome': 'error' if value.get('status') != 'success' else ('nonempty' if series else 'empty')}
    except (OSError, ValueError, urllib.error.URLError) as error:
        # Exception messages can contain endpoints or returned labels; retain the class only.
        return {'outcome': 'transport-error', 'error_class': type(error).__name__, 'seconds': time.monotonic() - before}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--url', required=True)
    parser.add_argument('--output', type=Path, required=True, help='New .jsonl.gz evidence file; parent must exist')
    parser.add_argument('--host', action='append', default=[], help='Repeat for selected hosts; omitted means fleet')
    parser.add_argument('--scope', choices=['host', 'group', 'fleet'], required=True)
    parser.add_argument('--select', action='append', default=[], help='Catalogue ID substring; repeat to select a cohort')
    parser.add_argument('--duration', type=int, default=900)
    parser.add_argument('--step', type=int, default=30)
    parser.add_argument('--deadline', type=int, default=25)
    parser.add_argument('--concurrency', type=int, choices=[1, 2, 4, 8], default=1)
    parser.add_argument('--repeats', type=int, default=1)
    parser.add_argument('--end', type=int, default=int(time.time()))
    parser.add_argument('--instant', action='store_true', help='Evaluate all selected expressions as alert instants')
    parser.add_argument('--plan', action='store_true', help='Describe request counts without querying or writing')
    args = parser.parse_args()
    if min(args.duration, args.step, args.deadline, args.repeats) <= 0:
        parser.error('durations, step, deadline and repeats must be positive')
    if (args.scope == 'host' and len(args.host) != 1 or args.scope == 'group' and len(args.host) < 2 or
            args.scope == 'fleet' and args.host):
        parser.error('scope must agree with host selection')
    end = args.end // args.step * args.step
    start = (end - args.duration) // args.step * args.step
    cases = workloads(args.select)
    if not cases:
        parser.error('no matching workload')
    plan = {'schema': 1, 'scope': args.scope, 'hosts_selected': len(args.host), 'start': start, 'end': end,
            'step': args.step, 'deadline': args.deadline, 'concurrency': args.concurrency,
            'repeats': args.repeats, 'cases': len(cases), 'instant_override': args.instant,
            'rate_scrape_interval_seconds': 1,
            'cache_state': 'uncontrolled live cache; repeated passes are warm candidates, not guaranteed cold/warm',
            'runner_sha256': hashlib.sha256(Path(__file__).read_bytes()).hexdigest(),
            'catalogue_sha256': hashlib.sha256((HERE / 'workload-catalogue.json').read_bytes()).hexdigest()}
    print(json.dumps(plan), flush=True)
    if args.plan:
        return
    parent = args.output.parent.resolve(strict=True)
    if not parent.is_dir() or args.output.suffixes[-2:] != ['.jsonl', '.gz']:
        parser.error('output must be a new .jsonl.gz file under an existing directory')
    cohort_started = time.monotonic()
    with args.output.open('xb') as destination, gzip.GzipFile(fileobj=destination, mode='wb', mtime=0) as compressed:
        def emit(value):
            compressed.write((json.dumps(value, allow_nan=False) + '\n').encode())
            compressed.flush()
            destination.flush()
        emit({'plan': plan})
        for repeat in range(args.repeats):
            def run(item, repeat=repeat):
                case, target = item
                expression = interpolate(target['expr'], args.host, args.step, args.duration)
                measured = request(args.url, expression, args.instant or target.get('instant', False), start, end, args.step, args.deadline)
                return {'id': case['id'], 'title': case['title'], 'expression_template': target['expr'], 'repeat': repeat,
                        'template_sha256': hashlib.sha256(target['expr'].encode()).hexdigest(), **measured}
            with ThreadPoolExecutor(max_workers=args.concurrency) as pool:
                futures = [pool.submit(run, case) for case in cases]
                for future in as_completed(futures):
                    record = future.result()
                    emit(record)
                    print(json.dumps({key: record.get(key) for key in ['id', 'repeat', 'outcome', 'seconds', 'series']}), flush=True)
        elapsed = time.monotonic() - cohort_started
        summary = {'completed_requests': len(cases) * args.repeats, 'seconds': elapsed,
                   'requests_per_second': len(cases) * args.repeats / elapsed}
        emit({'summary': summary})
        print(json.dumps({'summary': summary}), flush=True)


if __name__ == '__main__':
    main()
