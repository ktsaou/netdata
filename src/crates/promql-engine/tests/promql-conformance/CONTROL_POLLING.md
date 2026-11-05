# C control-state polling

The C control-state exports report cancellation/deadline status and remaining time. Previously an active deadline
caused two clock reads: one in `Control::check()`, followed by another for the duration. Netdata calls this interface
while reading storage points. A retained playground CPU profile confirmed the path through `rrdeng_reader_point`,
`promql_read_poll` and `netdata_promql_callback_control_state`.

The exports now use one timestamp for both status and duration. Cancellation is observed first and retains
precedence over expiry. No deadline means no clock read; a cancelled control with a deadline still reports remaining
time. This snapshot change kept every polling location; the raw-sample cadence below later changed them. Ordinary Rust `Control::check()` keeps its comparison-only path.
There is no new heap allocation, retained query state, ABI field, timer, limit or deadline policy.

The deterministic unit panel covers absent, future, equal and expired deadlines with and without cancellation,
counts clock invocations, and checks cancellation ordering. The C-interface unit panel exercises both owned and
borrowed exports. [control-poll-qualification.json](control-poll-qualification.json) binds the source, both build
profiles, unit results, strict Clippy and native C measurements. Broad expression and foreign-platform captures
remain attributed to their earlier sources; they were not repeated for this local snapshot change.

## Component measurement

Each run polls an actual C control two million times. These are medians of six alternating before/after pairs after
one warmup pair, on the development host. They measure the interface, not end-to-end query performance.

| Control | Release before → after | Size-optimized before → after |
| --- | ---: | ---: |
| No deadline | 10.22 → 10.83 ms | 30.30 → 29.73 ms |
| Active deadline | 122.58 → 67.51 ms | 147.34 → 90.92 ms |
| Expired deadline | 143.33 → 92.58 ms | 168.39 → 112.34 ms |
| Cancelled, deadline present | 66.24 → 66.23 ms | 91.26 → 90.14 ms |

The release no-deadline control is 0.61 ms slower per two million polls, about 0.30 ns per call. Cancelled controls
already used one clock read. The measured executable grows by 130 bytes of code in release and 96 bytes in
release-min, with unchanged data/BSS. Other query costs and the remaining live timeouts are separate findings.

To reproduce the component measurement on a supported Unix build, build the static library with the chosen profile,
then compile [control_poll.c](control_poll.c) against that exact archive and the matching header:

```sh
cargo build --locked --features fixtures --profile release --lib
cc -O2 -I include tests/promql-conformance/control_poll.c \
  target/release/libnetdata_promql_engine.a -ldl -lpthread -lm -o target/control-poll
./target/control-poll 1 2000000
```

Run from the crate directory; substitute the actual target directory if `CARGO_TARGET_DIR` is set. Modes 0/1/2/3
select no deadline, active deadline, expired deadline and cancelled-with-deadline respectively. Build and retain
both revisions before alternating runs. A component gain does not establish that a dashboard timeout is resolved.

## Raw-sample deadline cadence

Raw-sample walks previously read the deadline clock about three times per sample: in walk validation, in the
per-sample window and fold kernels, and through the C cursor's per-point dbengine poll.

- **Walk stride:** `walk_observed_inner` owns the deadline cadence. A per-page `ControlStride` makes a full check
  every 1024 samples. The existing full checks at walk entry, before each page and seek, and per output step remain.
- **Per-sample sites:** window loops, fold kernels, instant lookups and constant-time float copies call
  `check_cancelled()`. Rust cancellation is therefore still observed at the next sample.
- **Derived points:** subquery-derived points have no enclosing walk, so `derived_range` keeps a full check per point.
- **C cursor:** `cursor_next` polls before each page acquisition. The inner point loop reads at most one page
  without polling.
- **Accepted consequences:** an expired deadline can be noticed up to 1024 samples later; C cancellation can be
  noticed up to one page later. An invalid sample after a mid-page expiry can report its storage error instead of
  Deadline.

Test builds count deadline clock reads by call site. `raw_sample_walks_amortize_deadline_clock_reads` fails if
any site reads the clock more than once per four fetched samples across 19 range shapes;
`raw_sample_walk_observes_deadline_mid_walk` checks that a long walk still stops. Fixture clock reads (3 series ×
20,000 samples, 316 steps):

| Query | Before | After |
| --- | ---: | ---: |
| `rate(x[2m])` | 603,123 | 143,343 |
| `avg_over_time(x[10m])` | 2,423,331 | 143,391 |
| `quantile_over_time(0.5, x[10m])` | 23,330,185 | 438,337 |
| `mad_over_time(x[10m])` | 43,994,769 | 603,369 |

Remaining reads are per page, walk, step or label operation, not per raw sample. `count_over_time` prefix reads from
temporary backing still check per entry; they are computed values, not raw samples.

### Live measurement

Playground VM, serial, five passes per query at a fixed end time (`1793860680`), before and after deployment of
`214e633f64`. Before and after span an Agent restart.

| Query | Before median | After median | Ratio |
| --- | ---: | ---: | ---: |
| `sum(rate(system_net[2m]))`, 1h at 60s, 18 series | 0.722 s | 0.529 s | 0.73 |
| same, 15s step | 2.738 s | 2.002 s | 0.73 |
| same, 6h | 4.315 s | 3.134 s | 0.73 |
| `sum(avg_over_time(system_net[1h]))` | 4.307 s | 2.581 s | 0.60 |
| `sum(quantile_over_time(0.9, system_net[10m]))` | 14.136 s | 9.062 s | 0.64 |
| `sum by (host)(rate(cgroup_cpu[2m]))`, 194 series | 11.570 s | 8.909 s | 0.77 |

Result digests and fetched samples are identical for every query. Work counters differ by 172 to 10,282 units out
of 18 to 277 million, growing with steps and series; the change keeps every work increment and the pinned work
tests pass, so this is attributed to metadata changed across the restart (not verified). Repeated raw-sample reads
remain the dominant cost; fetched samples are unchanged by design.
