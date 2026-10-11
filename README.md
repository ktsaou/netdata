# Netdata PromQL engine

A standalone Rust PromQL evaluator with a versioned C interface. The library owns parsing, validation and
expression evaluation. Its caller supplies metadata, paged samples, temporary backing and output handling.

- Build: `cargo build --locked --release`.
- Prepare fixture backing: `mkdir -p tests/promql-conformance/.tmp`.
- Test: `cargo test --locked --features fixtures --release`.
- C interface: [include/netdata-promql.h](include/netdata-promql.h), ABI 3.
- Design: [approved decisions](tests/promql-conformance/DESIGN.md).
- Qualification: [conformance guide](tests/promql-conformance/README.md) and
  [source and binary cohorts](tests/promql-conformance/core-qualification.json).

The default feature profile follows the pinned stable Prometheus server. Optional parser and evaluator features
are explicit caller configuration. Raw samples stay paged across metrics and time. General exact quantiles use
caller-owned temporary backing. Resource accounting is enabled; enforcement is disabled by default.

The C boundary contains ordinary Rust panics. Build the library with `panic=unwind`, including size-optimized builds.
Rust 1.91 or later is required. Registry dependencies are locked; builds need an existing Cargo cache or network access.

[IMPORT.json](IMPORT.json) binds the extraction to the original Netdata source paths and hashes. Imported Git history
retains their earlier layout. Historical qualification keeps its actual source and binary identity; relocating a file
does not turn an earlier execution into a new qualification run. Netdata owns its live storage and HTTP integration.

The library is GPL-3.0-or-later. [NOTICE](NOTICE) and the accompanying licenses retain upstream attribution.
