# Journal metric lookup

DBengine's demand reader finds a metric UUID in a sorted journal directory.
Previously, every binary-search probe separately established and closed the
mapped-file access context. The lookup now establishes that context once for
the search. It still copies and compares the same records in the same order.

The directory's complete byte range is checked before searching. Empty
directories return without accessing the mapping. An unreadable mapped page
still fails the lookup, closes its context and leaves the caller responsible
for the same descriptor error and reference cleanup. Both demand reads and
promise expiration use this lookup.

This change adds no query state or allocation. The existing mapping reference
spans the search. The additional diagnostic site uses the existing per-thread
log rate limiter. Page-list searches, their cancellation callbacks, CRC checks,
file selection and sample decoding remain unchanged.

## Isolated qualification

Run `python3 tests/promql-integration/journal_lookup.py --cc gcc` or select Clang
with `--cc clang`. The runner extracts the actual lookup functions and journal
structs, and compiles them with the actual mapped-access macros and signal
handler. Only surrounding types, logging and mapping construction are supplied
by the fixture. Each execution retains its sources, executable and receipt in
an isolated `.tmp/journal-lookup-*` directory.

The fixture checks 2,372 outcomes, including empty, singleton and larger
directories, all present UUIDs and intervening absent keys, invalid geometry,
an unreadable page after a successful probe, enclosing-context cleanup, signal
mask restoration and a successful subsequent lookup. Complete returned-record
digests match the preceding implementation with both GCC and Clang.

The generated fixture also accepts a directory size for a bounded benchmark:
100,000 successful lookups with varied keys. Six alternating before/after blocks
were run for each compiler and size; block zero is warmup. These are lookup costs,
not complete-query latency:

| Compiler | Directory records | Before, ms | After, ms | Median paired ratio |
|---|---:|---:|---:|---:|
| GCC | 1 | 25.41 | 22.48 | 0.920 |
| GCC | 127 | 154.19 | 33.73 | 0.204 |
| GCC | 65,535 | 466.57 | 72.31 | 0.158 |
| Clang | 1 | 23.96 | 22.73 | 0.948 |
| Clang | 127 | 145.25 | 25.24 | 0.185 |
| Clang | 65,535 | 436.69 | 53.61 | 0.123 |

The paired ratio and separate absolute medians need not divide exactly. All
blocks, including warmup and singleton controls, are retained. Shared-host timing
variation and other query costs limit what these measurements establish.

The normal AMD64 Agent's compiled text increases by 412 bytes, initialized data
by 64 bytes, and BSS is unchanged. Its ELF thread-local memory increases from
14,136 to 14,200 bytes because of the additional diagnostic rate limiter.
This is 64 bytes per thread, not per metric or query.

The source-bound report is [journal-metric-lookup-qualification.json](journal-metric-lookup-qualification.json).
Full Agent storage/API checks and executable footprint are recorded separately
from the isolated fixture. Live playground results are reported in
[Grafana findings](grafana/FINDINGS.md).
