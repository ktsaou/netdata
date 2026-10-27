# C control-state polling

The C control-state exports report cancellation/deadline status and remaining time. Previously an active deadline
caused two clock reads: one in `Control::check()`, followed by another for the duration. Netdata calls this interface
while reading storage points. A retained playground CPU profile confirmed the path through `rrdeng_reader_point`,
`promql_read_poll` and `netdata_promql_callback_control_state`.

The exports now use one timestamp for both status and duration. Cancellation is observed first and retains
precedence over expiry. No deadline means no clock read; a cancelled control with a deadline still reports remaining
time. Every existing polling location remains. Ordinary Rust `Control::check()` keeps its comparison-only path.
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
