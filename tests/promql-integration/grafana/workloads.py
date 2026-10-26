#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""Generate the practical workload dashboards; existing four dashboards stay hand-maintained."""

import argparse
import json
from copy import deepcopy
from pathlib import Path

HERE = Path(__file__).resolve().parent
HOST = 'netdata_host_name=~"${host}"'
H = 'netdata_host_id,netdata_host_name'
APP = 'app_group'
I = H + ',netdata_chart_id'


def metric(name, match=''):
    return name + '{' + HOST + (',' + match if match else '') + '}'


def rate(name, match=''):
    return 'rate(' + metric(name, match) + '[$__rate_interval])'


def group(op, keys, expr):
    return f'{op} by ({keys}) ({expr})'


def definitions():
    """Panel expression is authoritative here; the catalogue references generated targets."""
    cpu = group('sum', APP, metric('app_cpu_utilization'))
    cpu_host = group('sum', H + ',' + APP, metric('app_cpu_utilization'))
    rss = group('sum', APP, metric('app_mem_usage'))
    busy = '(100 - ' + group('avg', H, metric('system_cpu', 'netdata_dimension_id="idle"')) + ')'
    past_busy = '(100 - ' + group('avg', H, metric('system_cpu', 'netdata_dimension_id="idle"') + ' offset 1h') + ')'
    used = metric('system_ram', 'netdata_dimension_id="used"')
    ram = metric('system_ram')
    mem_host = '100 * ' + group('sum', H, used) + ' / ' + group('sum', H, ram)
    fs = '100 * ' + group('sum', H + ',mount_point', metric('disk_space', 'netdata_dimension_id="used"')) + ' / ' + group('sum', H + ',mount_point', metric('disk_space'))
    traffic = rate('system_net')
    ingress = rate('system_net', 'netdata_dimension_id="InOctets"')
    egress = rate('system_net', 'netdata_dimension_id="OutOctets"')
    cpu_raw = metric('app_cpu_utilization')
    seen = group('count', H, metric('system_cpu', 'netdata_dimension_id="idle"'))
    earlier = group('count', H, 'count_over_time(' + metric('system_cpu', 'netdata_dimension_id="idle"') + '[1h])')
    app_seen = group('count', H + ',' + APP, metric('app_processes'))
    app_earlier = group('count', H + ',' + APP, 'count_over_time(' + metric('app_processes') + '[1h])')
    # title, expression, unit, question/interpretation, operator family
    return [
        ('fleet-bandwidth', 'Fleet Bandwidth', [
            ('Total ingress', f'sum({ingress})', 'Kbits', 'Sum of observed node ingress; internal fleet traffic is also included.', 'rate,sum'),
            ('Total egress', f'sum({egress})', 'Kbits', 'Sum of observed node egress; this is not unique traffic crossing a fleet boundary.', 'rate,sum'),
            ('Traffic by direction', group('sum', 'netdata_dimension_name', traffic), 'Kbits', 'Ingress and egress are separate totals; do not sum virtual and physical interface charts.', 'rate,group'),
            ('Ingress by node', group('sum', H, ingress), 'Kbits', 'Which nodes receive the bandwidth?', 'rate,group'),
            ('Egress by node', group('sum', H, egress), 'Kbits', 'Which nodes send the bandwidth?', 'rate,group'),
            ('Node share of egress', group('sum', H, egress) + f' / on() group_left sum({egress}) * 100', 'percent', 'Node contribution to total selected-node egress. A zero denominator stays undefined.', 'rate,join'),
            ('Interface errors across nodes', group('sum', 'netdata_dimension_name', rate('net_errors')), 'ops', 'Interface error events; physical and virtual interfaces may overlap.', 'rate,group'),
            ('Interface drops across nodes', group('sum', 'netdata_dimension_name', rate('net_drops')), 'ops', 'Interface drop events, not unique packets lost across the fleet.', 'rate,group'),
            ('Egress change versus one hour ago', f'sum({egress}) - sum(' + egress[:-1] + ' offset 1h))', 'Kbits', 'Absolute change at equal lookback windows one hour apart.', 'offset,rate'),
            ('Immediate egress rate by node', group('sum', H, 'irate(' + metric('system_net', 'netdata_dimension_id="OutOctets"') + '[$__rate_interval])'), 'Kbits', 'Last two counter points; more responsive and noisier than rate().', 'irate'),
            ('Top five sending nodes', 'topk(5, ' + group('sum', H, egress) + ')', 'Kbits', 'Ranking changes per step; holes from changing membership are expected here.', 'topk'),
            ('Average egress per reporting node', 'avg(' + group('sum', H, egress) + ')', 'Kbits', 'Mean node contribution, not total throughput.', 'nested-aggregation'),
            ('Network drops per observed packet', 'sum(' + rate('net_drops') + ') / sum(' + rate('net_packets', 'netdata_dimension_id=~"received|sent"') + ')', 'percentunit', 'Observed interface drop/packet ratio; received and sent exclude the separately reported multicast subset. Interfaces can overlap.', 'ratio,rate'),
            ('Nodes above 100 Mbit/s egress', 'sum(' + group('sum', H, egress) + ' > bool 100000)', 'short', 'Illustrative capacity threshold; choose the link-specific threshold before alerting.', 'bool,aggregation'),
        ]),
        ('fleet-applications', 'Applications Across Nodes', [
            ('CPU cores per application', f'({cpu}) / 100', 'short', 'User plus system CPU across nodes; 100 percent equals one core.', 'sum'),
            ('RSS per application', rss, 'mbytes', 'RSS across nodes in MiB. Shared pages can be counted in multiple processes.', 'sum'),
            ('Processes per application', group('sum', APP, metric('app_processes')), 'short', 'All observed processes in each application group across selected nodes.', 'sum'),
            ('Threads per application', group('sum', APP, metric('app_threads')), 'short', 'All observed threads by application group.', 'sum'),
            ('Nodes reporting each application', group('count', APP, app_seen), 'short', 'Reporting nodes, not a desired-replica inventory.', 'nested-aggregation'),
            ('Mean CPU cores per reporting node', group('avg', APP, cpu_host) + ' / 100', 'short', 'Average over reporting nodes only; absent nodes are not zero.', 'nested-aggregation'),
            ('Maximum CPU cores on one node', group('max', APP, cpu_host) + ' / 100', 'short', 'Largest observed node contribution for each application.', 'nested-aggregation'),
            ('Share of observed application CPU', f'{cpu} / on() group_left sum({cpu_raw}) * 100', 'percent', 'Share of observed application CPU, not share of total machine capacity.', 'join'),
            ('Fixed top ten application CPU histories', f'{cpu} and topk(10, ' + group('sum', APP, cpu_raw + ' @ end()') + ')', 'percent', 'Top applications by fleet CPU at the aligned range endpoint; actual gaps remain.', 'fixed-ranking,set'),
            ('Application disk I/O across nodes', group('sum', APP + ',netdata_dimension_name', 'abs(' + metric('app_disk_physical_io') + ')'), 'KiBs', 'Already a rate gauge; writes normalized for presentation.', 'abs,group'),
            ('Application page faults across nodes', group('sum', APP + ',netdata_dimension_name', metric('app_mem_page_faults')), 'ops', 'Already a rate gauge; minor and major faults remain separate.', 'group'),
            ('CPU with combined node/application display label', 'label_join(' + cpu_host + ',"display"," / ","netdata_host_name","app_group")', 'percent', 'Adds a display label while retaining original identity labels.', 'label_join'),
            ('RSS with copied application display label', 'label_replace(' + rss + ',"application","$1","app_group","(.+)")', 'mbytes', 'Copies a label for consumers expecting a different display field; identity is retained.', 'label_replace'),
            ('Nodes using more than one core per app', group('sum', APP, cpu_host + ' > bool 100'), 'short', 'Illustrative alert count across nodes for each application.', 'bool,nested-aggregation'),
        ]),
        ('fleet-capacity', 'Fleet Capacity', [
            ('Fleet RAM used ratio', f'100 * sum({used}) / sum({ram})', 'percent', 'Weighted by memory capacity; not an average of host percentages.', 'ratio,sum'),
            ('Fleet used RAM', f'sum({used})', 'mbytes', 'Observed used memory in MiB across selected hosts.', 'sum'),
            ('Fleet free RAM', 'sum(' + metric('system_ram', 'netdata_dimension_id="free"') + ')', 'mbytes', 'Free excludes cache and buffers; not Linux MemAvailable.', 'sum'),
            ('Fleet total RAM', f'sum({ram})', 'mbytes', 'Total of the collector RAM components.', 'sum'),
            ('Mean node CPU busy', f'avg({busy})', 'percent', 'Equal weight per reporting node, not a core-weighted fleet utilization.', 'nested-aggregation'),
            ('P95 node CPU busy', f'quantile(0.95, {busy})', 'percent', 'Cross-node distribution at each time; not a temporal percentile.', 'quantile'),
            ('Maximum node CPU busy', f'max({busy})', 'percent', 'Worst reporting node; busy includes I/O wait.', 'nested-aggregation'),
            ('Nodes above 80% CPU busy', f'sum({busy} > bool 80)', 'short', 'Illustrative threshold count; missing hosts remain absent.', 'bool'),
            ('Nodes above 90% used RAM', f'sum(({mem_host}) > bool 90)', 'short', 'Used-memory definition follows the collector, not available-memory pressure.', 'join,bool'),
            ('Three nodes with least free RAM', 'bottomk(3, ' + group('sum', H, metric('system_ram', 'netdata_dimension_id="free"')) + ')', 'mbytes', 'Per-step ranking of free memory; cached memory is not included.', 'bottomk'),
            ('Node CPU busy standard deviation', f'stddev({busy})', 'percent', 'Cross-node imbalance at each instant; nodes have equal weight.', 'stddev'),
            ('Filesystems above 90% used', f'sum(({fs}) > bool 90)', 'short', 'Counts observed mounts; bind mounts can refer to the same storage.', 'join,bool'),
        ]),
        ('history-comparisons', 'History and Forecasts', [
            ('CPU busy change versus one hour ago', f'{busy} - (' + past_busy + ')', 'percent', 'Percentage-point change, matched by node identity.', 'offset,join'),
            ('RAM change over 15 minutes', group('sum', H, 'delta(' + used + '[15m])'), 'mbytes', 'Gauge delta; do not interpret this as counter increase.', 'delta'),
            ('RAM slope over 15 minutes', group('sum', H, 'deriv(' + used + '[15m])'), 'suffix:MiB/s', 'Least-squares slope of used RAM.', 'deriv'),
            ('Used RAM linear prediction one hour ahead', group('sum', H, 'predict_linear(' + used + '[30m],3600)'), 'mbytes', 'Linear extrapolation, not a capacity guarantee; negative predictions are visible.', 'predict_linear'),
            ('Context switches over five minutes', group('sum', H, 'increase(' + metric('system_ctxt') + '[5m])'), 'short', 'Counter increase over each five-minute window.', 'increase'),
            ('Network counter resets over one hour', group('sum', H, 'resets(' + metric('system_net') + '[1h])'), 'short', 'Observed reconstructed counter resets; legacy/coarse history has documented limits.', 'resets'),
            ('Load value changes over five minutes', group('sum', H, 'changes(' + metric('system_load', 'netdata_dimension_id="load1"') + '[5m])'), 'short', 'Number of changes in the retained load gauge.', 'changes'),
            ('Five-minute CPU maximum', f'max_over_time(({busy})[5m:30s])', 'percent', 'Computed CPU sampled every 30 seconds; not all one-second spikes.', 'subquery,max_over_time'),
            ('Thirty-minute CPU P95', f'quantile_over_time(0.95, ({busy})[30m:1m])', 'percent', 'Temporal percentile of one-minute samples for each host.', 'subquery,quantile_over_time'),
            ('Minimum free RAM in 15 minutes', group('sum', H, 'min_over_time(' + metric('system_ram', 'netdata_dimension_id="free"') + '[15m])'), 'mbytes', 'Per-host low-water mark of free RAM.', 'min_over_time'),
            ('Last RAM change between two samples', group('sum', H, 'idelta(' + used + '[5m])'), 'mbytes', 'Last two gauge points, unlike the extrapolated full-window delta.', 'idelta'),
            ('RAM standard deviation over 15 minutes', group('sum', H, 'stddev_over_time(' + used + '[15m])'), 'mbytes', 'Volatility of the used-memory gauge.', 'stddev_over_time'),
        ]),
        ('data-availability', 'Data Availability', [
            ('Nodes reporting CPU now', f'count({seen})', 'short', 'Only CPU-reporting hosts within lookback; not all retained hosts.', 'count'),
            ('Nodes seen in the last hour', f'count({earlier})', 'short', 'Historical baseline for disappearance detection; not desired inventory.', 'count_over_time,count'),
            ('Nodes seen in last hour but missing now', f'{earlier} unless on(netdata_host_id) {seen}', 'short', 'Missing members remain labelled. If every host vanishes, historical data still identifies them.', 'unless,matching'),
            ('Applications seen in last hour but missing now', f'{app_earlier} unless on(netdata_host_id,app_group) {app_seen}', 'short', 'Detects disappeared host/application combinations, not expected deployments.', 'unless,matching'),
            ('Age of latest CPU sample by host', 'time() - ' + group('max', H, 'timestamp(' + metric('system_cpu') + ')'), 's', 'Only currently selectable samples; missing hosts require the separate disappearance query.', 'timestamp,time'),
            ('Oldest selectable CPU sample age', 'max(time() - ' + group('max', H, 'timestamp(' + metric('system_cpu') + ')') + ')', 's', 'Fleet maximum among reporting nodes.', 'timestamp,max'),
            ('CPU samples retained in five minutes', group('sum', H, 'count_over_time(' + metric('system_cpu', 'netdata_dimension_id="idle"') + '[5m])'), 'short', 'Counts actual points; no assumed collection frequency or fabricated coverage percentage.', 'count_over_time'),
            ('No CPU data anywhere in selection for five minutes', 'absent_over_time(' + metric('system_cpu') + '[5m])', 'short', 'A value of one signals complete selected-scope absence; empty is expected when data exists.', 'absent_over_time'),
            ('Reporting application groups per node', group('count', H, app_seen), 'short', 'Cardinality of reporting application groups.', 'nested-aggregation'),
            ('Nodes with any CPU point in five minutes', 'sum(' + group('max', H, 'present_over_time(' + metric('system_cpu') + '[5m])') + ')', 'short', 'Presence by node regardless of point count.', 'present_over_time'),
            ('Last observed CPU idle value in one hour', group('avg', H, 'last_over_time(' + metric('system_cpu', 'netdata_dimension_id="idle"') + '[1h])'), 'percent', 'Returns historical last values even if currently absent; this is deliberately not current health.', 'last_over_time'),
            ('Application CPU sample counts', group('sum', APP, 'count_over_time(' + metric('app_cpu_utilization') + '[5m])'), 'short', 'Counts user/system points across hosts, not unique hosts.', 'count_over_time,group'),
        ]),
        ('service-instances', 'Service Instances', [
            ('Nginx requests across instances', 'sum(' + rate('nginx_requests') + ')', 'reqps', 'Rate each independent counter before summing across hosts and instances.', 'rate,sum'),
            ('Nginx requests by instance', group('sum', I, rate('nginx_requests')), 'reqps', 'Host and chart identity distinguish collector instances.', 'rate,group'),
            ('Nginx connection states across instances', group('sum', 'netdata_dimension_name', metric('nginx_connections_status')), 'short', 'Reading, writing and idle connections are gauges.', 'sum'),
            ('HTTP response time by instance', group('max', I, metric('httpcheck_response_time')), 'ms', 'Latest measured response time; not request latency percentile.', 'max'),
            ('Worst HTTP response time', 'max(' + metric('httpcheck_response_time') + ')', 'ms', 'Maximum across reporting checks; missing checks are not successful.', 'max'),
            ('HTTP status states', group('sum', 'netdata_dimension_name', metric('httpcheck_status')), 'short', 'Counts the collector one-hot status dimensions; inspect failures separately.', 'sum'),
            ('Container CPU cores by instance', group('sum', I, rate('cgroup_cpu')) + ' / 100', 'short', 'Container CPU uses incremental dimensions; unlike app CPU it requires rate(). Nested cgroups may overlap.', 'rate,group'),
            ('Container CPU cores across observed cgroups', 'sum(' + rate('cgroup_cpu') + ') / 100', 'short', 'Sum of observed cgroup CPU; hierarchy can double-count, so this is not host total CPU.', 'rate,sum'),
            ('Scheduler histogram bucket observations', group('sum', 'netdata_dimension_name', rate('prometheus_grafana_local_go_sched_latencies_seconds')), 'ops', 'Collector stores disjoint histogram buckets without le. These are bucket rates, not cumulative Prometheus buckets.', 'rate,histogram-distribution'),
            ('Failed HTTP checks across nodes', 'sum(' + metric('httpcheck_status', 'netdata_dimension_id!="success"') + ')', 'short', 'Sum of mutually exclusive failure states among reporting checks.', 'sum,filter'),
            ('HTTP time in current failed state', metric('httpcheck_in_state') + ' * on(netdata_host_id,_collect_job) group_left ' + group('max', 'netdata_host_id,_collect_job', metric('httpcheck_status', 'netdata_dimension_id!="success"')) + ' > 0', 's', 'One-hot failed status gates state duration; a successful state is not an outage.', 'group_left,comparison'),
            ('Scheduler observations across instances', 'sum(' + rate('prometheus_grafana_local_go_sched_latencies_seconds_count') + ')', 'ops', 'Histogram count counters across instances. Native histograms and missing le projection are separate coverage gaps.', 'rate,histogram-count'),
        ]),
        ('database-instances', 'Database Instances', [
            ('Database transactions by instance', group('sum', I, rate('postgres_db_transactions_rate')), 'ops', 'Commit and rollback counters, converted to rates before grouping.', 'rate,group'),
            ('Fleet database transactions by outcome', group('sum', 'netdata_dimension_name', rate('postgres_db_transactions_rate')), 'ops', 'Outcomes remain separate across databases and nodes.', 'rate,sum'),
            ('Database connections across instances', 'sum(' + metric('postgres_db_connections_count') + ')', 'short', 'Observed database connections; no desired capacity inference.', 'sum'),
            ('Database connections by instance', group('sum', I, metric('postgres_db_connections_count')), 'short', 'Instance identity is retained instead of joining unrelated databases.', 'sum,group'),
            ('Connection utilization by node', group('max', H, metric('postgres_connections_utilization')), 'percent', 'Worst reported server utilization on each host.', 'max'),
            ('Worst server connection utilization', 'max(' + metric('postgres_connections_utilization') + ')', 'percent', 'Maximum, not sum, of percentages.', 'max'),
            ('Worst database cache miss ratio', 'max(' + metric('postgres_db_cache_io_ratio') + ')', 'percent', 'The collector dimension is cache MISS percentage, not hit percentage.', 'max'),
            ('Deadlocks across databases', 'sum(' + rate('postgres_db_deadlocks_rate') + ')', 'ops', 'Despite the context name, underlying dimensions are incremental counters.', 'rate,sum'),
            ('Awaited locks by instance', group('sum', I, metric('postgres_db_locks_awaited_count')), 'short', 'Observed awaiting lock modes summed per chart instance.', 'sum'),
            ('Fleet transaction rollback ratio', 'sum(' + rate('postgres_db_transactions_rate', 'netdata_dimension_name="rollback"') + ') / sum(' + rate('postgres_db_transactions_rate') + ')', 'percentunit', 'Ratio of summed rollback rates to summed commit-plus-rollback rates.', 'ratio,rate'),
            ('Database transaction share of fleet', group('sum', I, rate('postgres_db_transactions_rate')) + ' / on() group_left sum(' + rate('postgres_db_transactions_rate') + ')', 'percentunit', 'Each database instance contribution to selected-fleet transaction rate.', 'group_left,rate'),
            ('Active-query age distribution', group('sum', 'netdata_dimension_name', rate('postgres_queries_duration')), 'ops', 'Repeated observations of active query ages in disjoint buckets; not completed-query latency.', 'rate,distribution'),
        ]),
        ('alert-expressions', 'Alert Expression Workloads', [
            ('CPU busy alert candidates', f'{busy} > 80', 'percent', 'Filter comparison returns affected hosts and their actual values. Example threshold only.', 'comparison'),
            ('Memory alert candidates', f'({mem_host}) > 90', 'percent', 'Example condition; no rule hold duration or notifications are installed.', 'comparison,join'),
            ('Application CPU alert candidates across nodes', f'{cpu} > 400', 'percent', 'Application fleet CPU above four cores; alert identity is the application group.', 'comparison,sum'),
            ('CPU busy AND high memory', f'({busy} > 80) and on(netdata_host_id) (({mem_host}) > 90)', 'percent', 'Two conditions must hold on the same host.', 'and,matching'),
            ('CPU busy OR high memory flags', f'({busy} > bool 80) + on(netdata_host_id,netdata_host_name) (({mem_host}) > bool 90) > bool 0', 'short', 'Flags only hosts with both input metrics present; absent inputs are not treated as zero.', 'bool,join'),
            ('Two-window CPU alert candidates', f'(avg_over_time(({busy})[5m:1m]) > 80) and (avg_over_time(({busy})[30m:1m]) > 70)', 'percent', 'Illustrative sustained pressure over two explicitly sampled windows.', 'subquery,and'),
            ('CPU increase from one hour ago above 20 points', f'({busy} - (' + past_busy + ')) > 20', 'percent', 'Historical join; a missing historical counterpart produces no candidate.', 'offset,comparison'),
            ('Application nodes above two cores', group('sum', APP, cpu_host + ' > bool 200') + ' > 0', 'short', 'Per-app affected-node count; threshold is illustrative.', 'bool,group,comparison'),
            ('Missing nodes from recent baseline', f'{earlier} unless on(netdata_host_id) {seen}', 'short', 'Historical expected-member proxy; not a configuration-backed desired inventory.', 'unless'),
            ('Filesystem exhaustion projected within four hours', 'predict_linear(' + metric('disk_space', 'netdata_dimension_id="avail"') + '[1h],14400) < 0', 'gbytes', 'Illustrative linear prediction of available capacity crossing zero; bursty usage may invalidate the model.', 'predict_linear,comparison'),
            ('HTTP failures or complete absence', '(' + group('sum', 'netdata_host_id,_collect_job', metric('httpcheck_status', 'netdata_dimension_id!="success"')) + ' > 0) or absent(' + metric('httpcheck_status') + ')', 'short', 'OR retains failure identities and separately detects complete selected-scope absence.', 'or,absent'),
            ('HTTP checks slower than one second', group('max', I, metric('httpcheck_response_time')) + ' > 1000', 'ms', 'Per-instance candidates; empty can mean healthy or unavailable input, so pair with availability.', 'comparison'),
        ]),
    ]


def build():
    template = json.loads((HERE / 'fleet-overview.json').read_text())
    panel_template = template['panels'][4]
    result = {}
    for slug, title, definitions_ in definitions():
        dashboard = deepcopy(template)
        dashboard.update(uid='nd-pql-' + slug, title='Netdata / ' + title, version=0,
                         description='Practical multi-node PromQL workloads. Missing data stays missing; alert thresholds are examples.',
                         refresh='', panels=[])
        for number, (title, expr, unit, description, family) in enumerate(definitions_, 1):
            if slug == 'alert-expressions':
                description += ' Historical matches: the last legend value may precede the range endpoint.'
            panel = deepcopy(panel_template)
            panel.update(id=number, title=title, description=description,
                         gridPos={'x': ((number - 1) % 2) * 12, 'y': ((number - 1) // 2) * 8, 'w': 12, 'h': 8})
            defaults = panel['fieldConfig']['defaults']
            defaults.update(unit=unit)
            defaults.pop('max', None)
            defaults.pop('min', None)
            panel['targets'][0].update(expr=expr, legendFormat='')
            dashboard['panels'].append(panel)
        result[slug + '.json'] = dashboard
    return result


def catalogue(dashboards):
    families = {(slug, index): family.split(',') for slug, _, rows in definitions()
                for index, (_, _, _, _, family) in enumerate(rows, 1)}
    records = []
    for filename, dashboard in sorted(dashboards.items()):
        slug = Path(filename).stem
        for panel in dashboard['panels']:
            for target in panel.get('targets', []):
                if target.get('expr'):
                    records.append({'id': f"{dashboard['uid']}/{panel['id']}/{target['refId']}",
                                    'file': filename, 'panel': panel['id'], 'target': target['refId'],
                                    'title': panel['title'], 'families': families.get((slug, panel['id']), ['existing-regression']),
                                    'alert_expression': slug == 'alert-expressions',
                                    'empty_requires_input_check': True})
    return {'schema': 1, 'workloads': records, 'unqualified': [
        {'family': 'histogram_quantile/classic cumulative le', 'reason': 'Available collector histogram contexts expose disjoint buckets without le. Bucket-rate panels do not qualify cumulative histogram quantiles.'},
        {'family': 'native histogram storage', 'reason': 'Live DBengine adapter supplies floats only.'},
        {'family': 'alert rule scheduling', 'reason': 'Expression workloads do not test for-state, recording dependencies or notifications.'}]}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--check', action='store_true', help='Verify generated files without writing')
    args = parser.parse_args()
    generated = build()
    all_dashboards = {p.name: json.loads(p.read_text()) for p in HERE.glob('*.json') if p.name != 'workload-catalogue.json'}
    all_dashboards.update(generated)
    generated['workload-catalogue.json'] = catalogue(all_dashboards)
    for name, value in generated.items():
        text = json.dumps(value, indent=2) + '\n'
        path = HERE / name
        if args.check:
            if not path.exists() or path.read_text() != text:
                raise SystemExit(f'generated file differs: {name}')
        else:
            path.write_text(text)
    print(json.dumps({'dashboards': len(all_dashboards), 'workloads': len(generated['workload-catalogue.json']['workloads'])}))


if __name__ == '__main__':
    main()
