# Netdata PromQL engine

A standalone Rust PromQL evaluator with a versioned C interface. The library owns parsing, validation and
expression evaluation. Its caller supplies metadata, paged samples, temporary backing and output handling.

- Build: `cargo build --locked --release`.
- Prepare fixture backing: `mkdir -p tests/promql-conformance/.tmp`.
- Test: `cargo test --locked --features fixtures --release`.
- C interface: [include/netdata-promql.h](include/netdata-promql.h), ABI 4.
- Design: [approved decisions](tests/promql-conformance/DESIGN.md).
- Qualification: [conformance guide](tests/promql-conformance/README.md) and
  [source and binary cohorts](tests/promql-conformance/core-qualification.json).

The default feature profile follows the pinned stable Prometheus server. Optional parser and evaluator features
are explicit caller configuration. Raw samples stay paged across metrics and time. General exact quantiles use
caller-owned temporary backing. Resource accounting is enabled; enforcement is disabled by default.

Precise Rust and C requests preserve the pinned server's nanosecond request clocks alongside its millisecond
sample grid. Owned selector handles, time/duration parsing and numeric formatting let callers reuse the same
protocol semantics without retaining a parser tree or assembling a full query result.
Metadata discovery visits one validated series at a time, supplies the actual query account to provider/output callbacks,
and fetches no samples. Callers own selector unions and final ordering; a row can request successful early stop.
The limit parser retains pinned64-bit request behavior on every target. Report callbacks provide a borrowed account
for caller preparation and backed error bodies; independently owned allocation handles may outlive the report.
Callback-scoped index sorting reuses the pinned Go routine for backed output, preserving the order of rows with
equal displayed labels. Comparators read immutable caller metadata while Rust exclusively permutes the index array.
An allocation-free rune getter shares the pinned Go Unicode17 printability table with native protocol diagnostics.

Basic query statistics report actual execution phases, logical sample participation, useful source reads and
the peak of owned sample values. They use fixed-size observations; per-step arrays remain disabled.
The enlarged statistics layout requires rebuilding ABI 3 callers and checking the library version before use.

The C boundary contains ordinary Rust panics. Build the library with `panic=unwind`, including size-optimized builds.
Rust 1.91 or later is required. Registry dependencies are locked; builds need an existing Cargo cache or network access.

[IMPORT.json](IMPORT.json) binds the extraction to the original Netdata source paths and hashes. Imported Git history
retains their earlier layout. Historical qualification keeps its actual source and binary identity; relocating a file
does not turn an earlier execution into a new qualification run. Netdata owns its live storage and HTTP integration.

The library is GPL-3.0-or-later. [NOTICE](NOTICE) and the accompanying licenses retain upstream attribution.
