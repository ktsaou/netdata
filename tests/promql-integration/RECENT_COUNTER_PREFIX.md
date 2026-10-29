# Reusing a recent counter prefix

Counter queries can open successive cursors whose windows start inside the same
DBengine page. Previously, each cursor reconstructed the counter from the last
indexed page checkpoint. The provider now also retains one recent prefix: the
timestamp and exact counter value immediately before the first returned point of
a successful batch.

This is a fixed-size summary owned by the existing physical metric in the query.
It does not retain raw samples, another metadata catalogue, or another page.
The existing DBengine page cache and contexts references remain the owners of
data and metadata. Query-origin selection, units, resets, gaps, and legacy versus
anchored counter behavior remain unchanged.

## Correctness and lifetime

- Indexed checkpoint lookup still runs first, including its backing-file errors.
  A more recent eligible prefix can then replace that starting position.
- Backward seeks never use a prefix from their future.
- Publication happens only after a complete successful batch. Cancellation,
  end-of-stream, and a failure after partially decoding a batch preserve the
  previous recent prefix.
- Each cursor remembers whether it started from an interior recent prefix. That
  position must not become an indexed page checkpoint, even if another cursor
  changes the shared recent prefix before the first cursor reads.
- The prefix is copied when the existing checkpoint owner moves from inline to
  file-backed indexing. No pointer to the old owner survives a replacement.

On the measured AMD64 build, the added state is **16 bytes per counter checkpoint
owner and eight bytes per cursor**. These allocations use the existing query
accounting. This is not a universal 24-byte increase per query: a query may own
several metrics and cursors. Gauge cursors also gain the eight-byte field.

## Controlled measurement

The same neutral C callback workload runs against the preceding production code
and the candidate. Each of three dimensions contains 256 one-second points in a
single page: a native counter, a legacy counter, and a gauge. One provider query
and series stay open while 255 fresh cursors each return a moving two-point window.
Every returned timestamp and Float value is checked, as are end-of-stream and
provider/borrowed-memory release.

There are three alternating before/after blocks per codec, six repetitions per
dimension per block. Repetition zero is warmup. Timing covers cursor operations,
excluding collection and metadata selection. The 216 observations include warmups.

| Storage | Metric | Median before → after | Median paired block ratio |
|---|---|---:|---:|
| Array | Native counter | 1,939 → 1,105 µs | 0.576 |
| Array | Legacy counter | 1,690 → 708 µs | 0.427 |
| Array | Gauge | 680 → 688 µs | 1.027 |
| Gorilla | Native counter | 2,113 → 1,367 µs | 0.676 |
| Gorilla | Legacy counter | 1,865 → 815 µs | 0.449 |
| Gorilla | Gauge | 814 → 797 µs | 1.010 |

For both counter kinds and both codecs, provider reconstruction and storage-point
counts fall from **32,895 to 764**. Gauge reads remain **510**, with zero counter
folds. Accounted provider peak changes **36,516 → 36,540 bytes** for each counter
and **36,172 → 36,180 bytes** for the gauge.

These are short measurements on a shared workstation. Individual blocks vary:
the first array gauge block has a 1.655 ratio. All blocks remain in the evidence.
The measurements establish reduced provider reconstruction, not a 43-fold runtime
gain or an equivalent improvement in complete PromQL/Grafana queries. Native page
decoding and the rest of query execution still have costs.

## Qualification and reproduction

The canonical `storage.py --binary BUILD/netdata` runner executes both codecs and
requires all 18 moving-window rows in each read phase. The fixture is in
`prefix_cost_fixture()` in `src/web/api/promql/provider-test.c`; it deliberately
uses no new recent-prefix accessor, so it can also run with the earlier provider.
The separately expanded callback suite checks interleaved cursors, backward seeks,
signed legacy values, resets, reset gaps, mixed pages, cancellation, indexed backing
errors, and native/legacy failures partway through a batch. Mid-batch cancellation
was inspected through the publication path, not executed with a dedicated hook.

The [qualification report](recent-counter-prefix-qualification.json) records source
and executable identities, completed suites and the measurement scope. The
[compressed observations](recent-counter-prefix-observations.jsonl.gz) retain every
timing row, including warmups. Historical failed test-runner attempts are retained
privately and are not counted as passing qualifications.

No parser, Rust arithmetic, public ABI, storage format, streaming protocol, query
limit, or histogram policy changes are part of this optimization.
