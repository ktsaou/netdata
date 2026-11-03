# Context prefilters before dimension traversal

PromQL selection walks each host's contexts, instances and metrics. Previously every
metric of every context was visited, its UUID resolved and a sanitized metric-name
alias allocated, before the selector's `__name__`, `netdata_context` and
`netdata_host_id` equality/inequality predicates rejected it. These predicates now
run once per context, before its instance iterator opens; excluded contexts are
skipped entirely. `netdata_dimension_id` and `netdata_dimension_name` predicates
remain per metric, after the existing UUID-based physical reuse.

Name predicates still accept either the exact context name or its sanitized alias;
when several `__name__` predicates are present, one alternative must satisfy all of
them. Regex predicates remain downstream in normal matching. No index, cache or
persistent allocation is added; the temporary alias is now allocated once per
visited context instead of once per metric.

## Accepted behavior changes

- A series retained earlier in the same query whose chart moved to a context the
  selector excludes is no longer rediscovered through its UUID in that context.
  Retained series keep their original labels.
- Metadata identity errors inside excluded contexts no longer fail the query.
  Errors in matched contexts, host readiness checks and cancellation are unchanged.

## Qualification

The `context-prefilters` storage fixture (`src/web/api/promql/metadata-test.c`) runs
16 scans per codec: exact and alias names, alias collisions, contradictory and
inequality name predicates, context and host predicates, dimension rejection,
included and excluded identity errors, readiness before filtering, cancellation,
and a real `rrdinstance_from_rrdset()` chart migration with retained labels. The
previous traversal fails exactly the two accepted-change scans and passes the
other 14. The current build passes all 16 with zero remaining provider or view
memory.

Internal-check builds pass all eight storage phases on both codecs and the 154
actual HTTP checks. A normal build compiles and links; its storage read phase
reports 414 rows with no errors, including `context-prefilters` 16/16. The storage
and HTTP runners require internal-only fixtures, so they do not complete against a
normal build.

## Live measurements

Serial queries on the playground at a fixed evaluation time (`time=1793557800`),
five passes each, before deployment and after it (after a short settle period):

| Query | Before median | After median | Provider-held allocations |
|---|---:|---:|---:|
| `sum by (...) (rate(postgres_db_transactions_rate{netdata_host_name=~".+"}[31s]))` | 5.043 s | 0.006 s | 205.4 MB → 1.1 MB |
| `system_cpu{netdata_host_name="promql"}` | 5.051 s | 0.021 s | 208.1 MB → 3.8 MB |
| `{netdata_context="system.cpu",netdata_host_name="promql"}` | 4.376 s | 0.022 s | 3.9 MB → 3.9 MB |
| `vector(1)` control | 1.2 ms | 0.8 ms | unchanged |

Results, samples fetched and work counters are identical before and after for
every query. Before and after span an Agent restart, so they are not a paired
comparison. To separate the restart from the change, selectors that still visit
every metric were timed on the restarted new build: `{netdata_dimension_id=...}`
alone takes 4.3–5.3 s, so full traversal remains expensive after the restart and
the speedup comes from skipping contexts.

That residual cost is the next metadata bottleneck: selectors without a name,
context or host predicate still visit every metric. The playground's largest
streamed host holds about 3.6 million metrics in 1.57 million instances. A regex
`__name__` selector over all hosts still exceeds the 25-second deadline. These are
not addressed here.

[context-prefilters-observations.jsonl](context-prefilters-observations.jsonl)
retains the per-pass measurements.
