# PromQL engine: design decisions

The native query engine is a standalone Rust implementation of the pinned PromQL language with a typed C interface.
It evaluates raw storage through bounded pages and replay, supports streaming and owned results, and accounts for
resources with enforcement disabled. Live dbengine/API integration, the independent rule engine and Go Cloud
execution are subsequent stages.

This document extracts the approved design and its consequences. It records the completed core at code checkpoint
`e5a9338d766eea359425641e1539760fca33c491` and qualification checkpoint
`5d1cb3c03caf202535161321ccde9f41b5803dc6`; it introduces no new runtime policy.

## Implementation status

| Status | Scope |
| --- | --- |
| Completed standalone core | Owned parser/type checker, float and native-histogram values, selectors, time/range/subquery evaluation, operators, joins, aggregations, stable and explicitly enabled optional functions, diagnostics, typed storage/output interfaces, C ABI 3, accounting and cooperative control. |
| Qualified evidence | Pinned upstream and required SRE expression panels, source-bound native/foreign checks, ownership/control regressions and actual C/C++ embedding. These finite checks do not prove equivalence for every possible input. |
| Approved later integration | Existing retained Netdata data, dual metric names, endpoint/tier policy, Agent/Parent APIs, Go Cloud distribution and a meta health engine parallel to existing health. |
| Designs still open | Production identity mapping and consistency, distributed plan/transport, API behavior, rule configuration/lifecycle/ownership, adaptive enforcement and broader platform/rollout policy. |
| Excluded | Alertmanager implementation and compatibility. |

The [README](README.md#standalone-expression-core) summarizes implementation and qualification.
[contract.json](contract.json) records the comparison/runtime contract;
[core-qualification.json](core-qualification.json) preserves actual source/binary cohorts and limitations.

## Product and implementation direction

### 1. One query language across Agent, Parent and Cloud

- **Choice:** provide complete PromQL for API queries and internal alert expressions on all three roles. Native
  Agent/Parent execution uses Rust through a C interface; the Cloud implementation must be Go.
- **Reason:** the query and alert paths need the same expression semantics, including matching, time, histograms,
  diagnostics and threshold outcomes.
- **Consequence:** Go Cloud must share the semantic contract. The division of processing between native executors
  and Cloud remains a design discussion; the core's local scheduler does not select that distributed split.
- **Status:** native expression core delivered; production and Cloud integration remain later work.

### 2. A parallel meta health engine; Alertmanager excluded

- **Choice:** build a rule engine outside chart/component health, running alongside existing health. It must work
  on Agents, Parents and Cloud. Alert routing and notification features remain Netdata product concerns.
- **Reason:** arbitrary PromQL expressions and fleet scopes should not require chart-bound alert definitions.
- **Consequence:** rule loading, recording outputs, state identity, timers, persistence, reload/failover and delivery
  need explicit integration contracts. Alertmanager exclusion does not settle Prometheus rule-file compatibility.
- **Status:** required expression evaluation is delivered; the rule lifecycle and notification integration are later stages.

### 3. Choose Rust after comparable C, C++ and Rust experiments

- **Choice:** preserve all three native prototypes and complete the modular engine in Rust with a C interface.
- **Reason:** gradual migration of Netdata to Rust is a project objective. The experiment established working
  implementations and a C boundary, then profiled and reduced confirmed Rust costs before selecting it.
- **Consequence:** native ownership benefits come with a Rust toolchain, C-boundary qualification and ongoing
  compatibility maintenance. Prototype timings and optimization flags are historical measurements; they do not
  establish completed-core performance or select production packaging flags.
- **Experiment scope:** the approximate common dashboard/alert slice was an 80/20 feasibility threshold, not a
  measured percentage of monthly traffic. Full language and SRE completion replaced that prototype scope.
- **Evidence:** [comparative prototypes and performance study](../promql-prototype/README.md).

## Language and compatibility contract

### 4. Pin upstream behavior and keep feature profiles explicit

- **Choice:** target Prometheus v3.15.0, module `v0.315.0`, commit
  `5241a27fe3c6983549fccc32f6e65917408c63cd`, with the retained Go 1.27.1 / Unicode 17.0.0 reference.
- **Profiles:** `server-default` enables stable language behavior, duration expressions, `@` and negative offsets. Optional functions,
  fill modifiers, extended range selectors, delayed name removal and start-timestamp evaluation remain disabled
  unless explicitly enabled. `upstream-tests` records the options used by the complete builtin test corpus.
- **Reason:** upstream release, parser options, Go arithmetic and Unicode behavior all affect observable results.
- **Consequence:** optional support is implemented and gated, rather than silently folded into server defaults.
  Upstream upgrades require deliberate source/profile qualification.
- **Evidence:** [reference and profiles](README.md#reference-and-profiles), [language catalog](language-catalog.json).

### 5. Own the byte lexer, Pratt parser and ordered typed validator

- **Choice:** own the frontend: byte-safe lexical analysis, explicit-stack Pratt parsing, a flat typed AST with
  source spans, and a separate validator following upstream validation/error order.
- **Reason:** the inspected parser alternative would require a compatibility fork for byte escapes, diagnostics,
  spans, profiles and newer grammar. Owning these layers makes the full pinned contract explicit.
- **Consequence:** parsing does not depend on recursive expression depth fitting the call stack, but grammar,
  diagnostics and upstream drift become maintenance responsibilities. Strings and labels preserve their byte
  semantics rather than undergoing blanket UTF-8 conversion.
- **Evidence:** [owned frontend](README.md#owned-rust-frontend), [frontend source](../../src/crates/promql-engine/src/frontend).

### 6. Own Go regex compatibility; reuse a Rust execution backend

- **Choice:** own Go-compatible syntax, byte/Unicode handling, captures, replacement expansion and selector
  optimizations. Use pinned `regex-automata` and `regex-syntax` as execution/encoding components.
- **Reason:** ordinary Rust regex or C++ RE2 behavior alone does not reproduce Go's ASCII `\w`, duplicate capture
  names or replacement-reference rules such as `$1x`. The backend does not define the accepted grammar.
- **Consequence:** this avoids writing a complete matcher from scratch while retaining responsibility for the
  compatibility frontend, generated Unicode data and dependency changes. Regex allocation estimates have the
  separate limitations described under decision 17 below.
- **Evidence:** [Go-compatible regex frontend](README.md#owned-go-compatible-regex-frontend),
  [pinned dependencies](../../src/crates/promql-engine/Cargo.toml).

### 7. Preserve upstream arithmetic and exact expression outcomes

- **Choice:** use upstream numerical algorithms and operation order. Require exact value kinds, byte labels and
  strings, timestamps, series membership, discrete/boolean outcomes, diagnostic sets and error attribution.
- **Numeric test contract:** use upstream `almost.Equal` with epsilon `1e-6` and its near-zero/NaN rules for
  numeric fields; histogram boundaries are exact and equivalent packing follows `Compact(0)`. Report every paired
  binary64 difference independently of the tolerance result.
- **Reason:** a one-bit intermediate difference can change whether an alert threshold matches. Tolerated numeric
  closeness cannot excuse a different boolean, selected series or error.
- **Consequence:** tolerance is a comparison rule, not a runtime rounding/normalization step. Algebraic rewrites
  or reassociated reductions need semantic qualification rather than a real-number equivalence argument.
- **Evidence:** [numerical contract](contract.json), [comparison owner](compare.py).

### 8. Fix Go AMD64, including its retained FMA-on route, across devices

- **Choice:** use the retained AMD64 arithmetic and semantic integer behavior on every supported target, refined
  to its FMA-on route. Preserve observable NaN operand destinations, signed zero and source conversion behavior.
- **Reason:** architecture and CPU-feature differences can change histogram equality and boolean thresholds;
  matching each device's local Go route would produce device-dependent expression outcomes.
- **Consequence:** portable arithmetic/conversions can require more work than host primitives, including software
  fused arithmetic. Runtime CPU features must not select different semantics. Semantic integer behavior stays
  canonical while physical slice/allocation indexes remain checked, target-sized indexes.
- **Evidence:** [canonical numerical behavior](README.md#canonical-numerical-operations-and-semantic-integer-widths),
  [numerical source](../../src/crates/promql-engine/src/numerical.rs).

### 9. Define deterministic order only where upstream leaves it unspecified

- **Choice:** at source-unspecified intermediate map-assembly boundaries, order by full label bytes. Preserve
  source-specified traversal, sorting operations and provider order elsewhere.
- **Reason:** identical Go requests can produce different reduction and boolean results at those unspecified
  boundaries. A fixed policy makes Netdata's answers reproducible.
- **Consequence:** this is an explicit compatibility policy, not a claim that every raw oracle order matches.
  Keep original order/membership/bit disagreements and qualify policy-dependent outcomes separately.
- **Evidence:** [ordering and compatibility report](core-qualification.json), [numerical contract](contract.json).

### 10. Preserve value fidelity and keep float slots compact

- **Choice:** retain raw binary64 bits, the distinct stale marker, start timestamps, native histogram schemas,
  spans/buckets/custom bounds, reset hints and declared optional-array presence. Owned histogram payloads are boxed;
  ordinary float slots stay compact.
- **Reason:** blanket normalization would lose source-observable semantics. Inline histogram ownership fields
  would inflate every float value and output point.
- **Consequence:** histogram values pay an additional accounted allocation/indirection. Metadata and histogram
  payloads can still grow; compact float storage is not a total-memory bound. Rust value layout is separate from
  the public typed C layout.
- **Evidence:** [value model](../../src/crates/promql-engine/src/value.rs),
  [owned values and output](../../src/crates/promql-engine/src/result.rs).

### 11. Record two narrow upstream compatibility exceptions

| Choice | Reason and boundary |
| --- | --- |
| A specific error when reached `changes()`/`resets()` float/histogram heads share a timestamp and the source merge cannot advance. | Avoid reproducing nontermination without inventing float-first semantics. Earlier acquisition and errors keep precedence; this is not a rejection of all mixed samples. |
| Reject each finalized/provider label name or value of at least `2^24` bytes. | The pinned encoder corrupts the exactly-16 MiB boundary. Apply representation validity per field, preserving source phases and provisional label operations; this is not an enabled memory/cardinality cap or a total-labelset limit. |

Original failures remain evidence. [Approved exceptions](contract.json) and
[label encoding](README.md#label-encoding-compatibility-exception) record the exact scope.

## Execution and memory

### 12. Consume bounded pages; retain operator state and replay

- **Choice:** never require a whole metric's raw history, or all selected raw metrics at one instant, to fit RAM.
  Read metrics through bounded cursor pages, fold sufficient state, and replay when exact evaluation requires it.
  The provider declares its page bound; the core validates returned pages against that contract.
- **Reason:** dbengine datasets can exceed RAM along either the time or series axis.
- **Consequence:** replay trades CPU/I/O for bounded raw-input retention. Descriptors, groups, current histogram
  payloads, diagnostics and an owned complete result may grow with cardinality; raw paging does not guarantee
  bounded total query memory or RSS, especially with enforcement disabled.
- **Cache policy:** the live adapter will reuse dbengine's existing shared LRU. The core introduces no second
  raw-history cache. Its completed-fold cache replaces earlier clocks and retains scalar progress certificates,
  rather than raw timelines.
- **Evidence:** [typed storage](../../src/crates/promql-engine/src/storage.rs),
  [fold cache](../../src/crates/promql-engine/src/range_cache.rs), [memory contract](contract.json).

### 13. Exact temporal sorting uses provider-owned temporary backing

- **Choice:** general temporal quantiles/provenance use exact float-bit slots and literal upstream sorting over
  provider-managed backing, with two 1,024-value RAM pages per active operator, bounded controller state and
  canonical 64-bit logical positions.
- **Reason:** exact sort-dependent outcomes cannot use an approximate quantile or a full raw-window RAM array.
- **Consequence:** the provider chooses the backend and cleanup policy. The core creates no files and has no
  whole-history RAM fallback. Missing, full or failed backing produces a specific storage error when that path is
  reached. Diskless providers need another suitable backend or return that error.
- **Accounting:** logical extent and read/write traffic are separate from RAM, RSS and allocated filesystem blocks.
  Production backing policy belongs to the later provider integration.
- **Evidence:** [temporary interface](../../src/crates/promql-engine/src/storage.rs),
  [paged float-bit owner](../../src/crates/promql-engine/src/temporary_float_bits.rs).

### 14. Stream certified common chains; retain general replay for other shapes

- **Choice:** certify common single-selector expressions and evaluate their original child ticks through one
  cursor/page using the ordinary operator machine. Keep the original grid, clocks, flags and diagnostics.
- **Reason:** repeatedly reconstructing earlier prefixes was prohibitively expensive even with bounded RAM.
- **Boundary:** certification needs one physical ordinal per row, unique projected full-label hashes, suitable
  scalar parameters and compatible clocks. Ambiguous projections, joins, aggregations, sorting, INFO, nested
  matrix children and vector-derived scalar parameters retain ordinary replay.
- **Consequence:** those are optimization boundaries, not unsupported language features. Nested replay can still
  multiply work; this is not a general distributed planner or a universal performance guarantee.
- **Evidence:** [single-selector schedules](README.md#common-single-selector-schedules),
  [certification owner](../../src/crates/promql-engine/src/scheduled_expression.rs).

### 15. Advance within the immutable raw page already held

- **Choice:** scheduled instantaneous forward movement reuses its current page. Validate the unread tail before
  the first logical advance; perform physical Seek/Next when movement beyond the page requires them.
- **Reason:** discarding and refetching the same page at each step caused billions of repeated sample reads.
- **Consequence:** preserve page validation, earlier visitor errors and errors from actual operations reached.
  Removing a redundant callback also removes errors tied only to that callback; general range-scan semantics remain intact.
- **Measured witness:** the retained 100,000-point fixture fell from 7.44 billion fetched entries / 151.65 seconds
  to 7.3 million / 16.49 seconds, with flat accounted workspace. These are original fixture-cohort measurements,
  not dbengine/device throughput guarantees or total-RAM measurements.
- **Evidence:** [shared cursor replay](README.md#shared-instantaneous-cursor-replay),
  [paging driver](../../src/crates/promql-engine/src/storage.rs).

## Runtime, ownership and accounting

### 16. Observe resources now; keep every resource cap disabled

- **Choice:** account current/peak owned and query-held borrowed bytes, sample/work totals, retained series/groups,
  output/diagnostics, active/peak executions and elapsed time. Keep memory, sample/work, regex, cardinality, output,
  concurrency-admission and implicit engine-timeout enforcement disabled.
- **Reason:** one fixed profile would be excessive for small devices and restrictive for large Parents. The
  user explicitly replaced the earlier budget-enforcement proposal with accounting-only operation.
- **Consequence:** observations support later adaptive policy, but no adaptive formula, admission queue or fixed
  defaults are selected. Actual allocator OOM may remain process-fatal. Caller-requested cancellation/deadlines
  remain independent controls; representation/provider validity checks still apply.
- **Evidence:** [accounting](../../src/crates/promql-engine/src/accounting.rs), [runtime contract](contract.json).

### 17. Precise observable accounting; explicit regex dependency estimates

- **Choice:** precisely charge observable engine-owned requested allocations and borrowed views by ownership and
  lifetime. Keep the qualified matcher and explicitly estimate opaque regex/backend and selector-table memory.
- **Reason:** the dependency's memory reports omit some capacities and temporary/high-water allocations; replacing
  the matcher solely to make those internals exact was not selected.
- **Consequence:** totals containing estimates are not exact allocator totals or guaranteed memory bounds. Allocator
  overhead and process RSS are different observations. Foreign allocations need provider reports/handles; the core
  cannot infer unreported memory. These limitations remain visible rather than hidden in a supposedly exact total.
  Rust keeps its normal allocator; accounting does not replace the process allocator.
- **Evidence:** [runtime accounting contract](contract.json),
  [regex backend accounting](../../src/crates/promql-engine/src/frontend/regex_nfa.rs).

### 18. One precisely accounted shared owner; explicit cancellation attribution

- **Choice:** use one local strong-only shared wrapper with known allocation layout and structural Box trait
  erasure. Count one physical allocation across aliases and release its charge after final payload/deallocation.
- **Reason:** payload-only accounting misses shared headers/ownership allocations. A uniform owner covers bounds,
  matchers, diagnostics, selections and other shared engine state.
- **Consequence:** reference counting/layout code requires focused qualification. This wrapper does not add Weak
  or copy-on-write support. Engine-created cancellation allocations remain charged to their creator, including
  cross-engine use; caller/default tokens remain external and query-held references are observed separately.
  Cycle-free accounting roots avoid leases retaining their own account indefinitely.
- **Evidence:** [shared owner](../../src/crates/promql-engine/src/shared.rs),
  [control ownership](../../src/crates/promql-engine/src/storage.rs), [accounting roots](../../src/crates/promql-engine/src/accounting.rs).

### 19. Query-owned allocation handles cover temporary C memory and cleanup

- **Choice:** C callbacks receive the actual query's borrowed accounting capability and acquire/resize/release
  allocation handles. Handles retain that query's account independently; they observe foreign requested bytes and
  do not allocate or free the foreign storage themselves.
- **Reason:** an allocation freed before a callback returns is invisible in a held-memory snapshot. Handles capture
  transient peaks and overlapping allocations.
- **Consequence:** separate overlapping allocations need separate handles. Snapshots exclude memory already charged
  through handles or descendants. Normal callbacks obtain accounting through control; resource/close/abort callbacks
  receive it directly, including abort-before-begin for a valid query. An invalid call that never establishes a query
  has no fabricated account. Charges persist through actual cleanup.
- **Evidence:** [C allocation interface](../../src/crates/promql-engine/include/netdata-promql.h),
  [provider bridge](../../src/crates/promql-engine/src/c_abi/provider.rs),
  [output cleanup](../../src/crates/promql-engine/src/c_abi/output.rs).

### 20. Typed cursors and one coherent C ABI 3

- **Choice:** expose opaque engine/query/result/report/control handles, counted byte views, tagged samples,
  complete typed histograms and explicit paired ownership. Providers may conservatively prefilter; Rust applies
  actual matcher semantics. Selection, series, cursor and query scopes close through their declared lifecycle.
- **Reason:** typed cursors avoid caller materialization of the complete query dataset and define callback lifetimes.
- **Consequence:** borrowing, successful NULL tokens and failed-open cleanup are explicit contracts. Headers,
  adapters, callers and observations migrated together to ABI 3; incompatible versions are rejected and ABI 2
  consumers must rebuild. Optional temporary callbacks are supplied as one complete group or absent until needed.
  Callers check ABI version before using unprefixed observation layouts.
- **Concurrency:** immutable compiled queries have independent execution contexts; cancellation may be concurrent.
  Access/release of a handle must not race. Getter views belong to their live result/report owner.
- **Evidence:** [C header](../../src/crates/promql-engine/include/netdata-promql.h),
  [checked ABI](../../src/crates/promql-engine/src/c_abi/checked.rs).

### 21. Synchronous provisional chunks plus an independently owned collector

- **Choice:** execute callbacks synchronously on the caller's thread. Borrowed chunks remain provisional until
  final commit; failure aborts output. Consumers can release chunks promptly. An accounted owned collector can
  retain the complete answer until its own release.
- **Reason:** output itself can exceed input working memory; streaming gives API/internal consumers a way to avoid
  retaining the whole answer while collection remains useful for callers needing independent ownership.
- **Consequence:** there is no post-return provider access, output spool or selected HTTP publication policy.
  Owned collection deliberately grows with the answer and does not inherit the raw-page memory guarantee.
- **Evidence:** [output model](../../src/crates/promql-engine/src/result.rs),
  [C output bridge](../../src/crates/promql-engine/src/c_abi/output.rs).

### 22. Contain ordinary Rust panics; preserve source failure semantics

- **Choice:** catch ordinary unwinding at the C boundary, discard the affected execution context and return an
  Internal error. Require panic-unwind builds, including the standalone `release-min` profile; install no process
  runtime/executor/hooks. The standalone crate uses Rust edition 2024 and MSRV 1.91, outside the Agent's default build.
- **Reason:** a Rust engine bug should not unwind into a C caller or require a process-wide runtime change.
- **Consequence:** allocator abort, invalid foreign pointers, foreign unwinding/longjmp and double-panic abort are
  outside ordinary panic containment. Predictable reached upstream native faults retain their Runtime/WrappedRuntime
  semantics; they are not silently converted to arbitrary successful values or confused with Rust bugs.
- **Evidence:** [crate profiles](../../src/crates/promql-engine/Cargo.toml),
  [unwind guard](../../src/crates/promql-engine/src/lib.rs), [C containment](../../src/crates/promql-engine/src/c_abi/mod.rs).

### 23. Cooperative control preserves failure and diagnostic precedence

- **Choice:** poll explicit cancellation/deadlines and record work through variable task, metadata, label-byte,
  parenthesis and diagnostic walks. A completed provider/output failure retains precedence over later cancellation.
  Successful/END callbacks poll before diagnostic conversion; position scans still charge work on failure paths.
- **Reason:** paging alone does not stop long metadata/preprocessing work, and cancellation must not replace an
  already completed source failure or publish an incomplete diagnostic.
- **Consequence:** formatting uses accounted candidate storage before publishing a complete message; successful
  output commit has no later fallible setup. Controls do not preempt a blocking foreign callback.
- **Evidence:** [control/work observations](README.md#cooperative-control-and-work-observations),
  [annotation owner](../../src/crates/promql-engine/src/annotations.rs).

## Approved data policies awaiting the production adapter

### 24. Query the decoded retained Netdata dataset

- **Choice:** existing collectors, Prometheus profiles, NIDL entities and decoded retained values are authoritative.
  Evaluate complete PromQL over those inputs; do not add a separate original-exporter sample store.
- **Reason:** Netdata already collects and structures these metrics. The new language should use that existing data.
- **Consequence:** collector transformations, compression/downsampling and retained statistics remain meaningful.
  Language compatibility cannot reconstruct original values, gaps, timestamps or histogram meaning lost before evaluation.
- **Status/evidence:** approved production direction; the historical
  [dbengine replay prototype](../promql-prototype/README.md#implementations-and-input-contract) demonstrates a limited
  retained-data path. The completed typed core does not include the live NIDL/dbengine adapter.

### 25. Expose exact and underscore names as distinct logical series

- **Choice:** expose both exact context names and underscore forms, such as `system.cpu` and `system_cpu`, without
  a `netdata` prefix. They share stored samples but remain ordinary distinct query-visible `__name__` values.
- **Reason:** support Netdata's existing context vocabulary and conventional PromQL identifiers simultaneously.
- **Consequence:** selectors matching both names return both logical series; aggregates may count both and functions
  removing metric names may cause ordinary duplicate-label errors. There is no special alias deduplication. Dotted
  names use standard quoted/string-selector syntax, not a grammar extension.
- **Still open:** broader normalization, collisions between contexts and underscore names, exact identity-label
  keys, native-label collisions and histogram grouping.
- **Evidence:** [NIDL model](../../docs/NIDL-Framework.md),
  [retained-data prototype contract](../promql-prototype/README.md#implementations-and-input-contract).

### 26. Endpoint means, finest-tier ownership and ordinary lookback

- **Choice:** expose numeric nonempty retained records as `(end_time_s * 1000, sum/count)`. Freeze each metric's
  per-tier retention spans before clipping the query window; the finest retained span owning an endpoint wins.
  A missing fine-tier record remains a gap rather than triggering coarse fallback.
- **Reason:** use actual completed retained records without dashboard interpolation or synthetic timestamps.
- **Consequence:** keep/discard whole records by endpoint, including records crossing a tier seam; do not trim,
  reweight or retimestamp them. Omit generic gaps without fabricating Prometheus stale markers; exclude uncompleted
  virtual accumulators and preserve read failures as errors. Ordinary range functions equally weight exposed
  samples, so coarse means do not recover original maxima or time-weighted histories.
- **Lookback:** ordinary configurable PromQL lookback, initially five minutes. No automatic holding over coarse
  intervals; sparse historical samples can yield sparse instant results.
- **Still open:** concrete API/rule lookback configuration, concurrent-expiry consistency, fixed Agent/Parent source
  selection and histogram projection. Frozen tier ownership alone does not provide transactional isolation.
- **Evidence:** [retained point model](../../src/libnetdata/storage-point.h),
  [storage cursor interface](../../src/database/storage-engine.h),
  [prototype point projection](../promql-prototype/README.md#implementations-and-input-contract).

## Qualification and remaining decisions

### 27. Independent pinned expectations and every required SRE expression

- **Choice:** capture original upstream commands, computed lexer/parser cases, effective options, complete inputs/
  outputs/errors and original assertions. Bind source, runtime, inputs and candidate binaries; preserve original
  generated-range assertion scope. Rust output must not generate its own expected results.
- **Required workload:** all pinned SRE alert and recording expressions plus infra issue 6028, exercised with
  meaningful controlled inputs, matches/absence/time/threshold cases and candidate-owned recording dependencies.
  The inventory has 790 total source slots, 1,110 rendered expressions and 608 SLO recording expressions; these
  are not 790 recordings or a unique-query/traffic denominator. Private configuration stays local.
- **Consequence:** unsupported required cases cannot count as passes. Keep original failures, numeric declarations,
  raw policy disagreements and each cohort's actual source/binary identity. Catalog AST witnesses and overlapping
  counts do not establish node visitation, branch coverage or equivalence for every possible input.
- **Current evidence:** 471 native units in each standalone build, three GNU targets, finite full-language/SRE
  panels and actual C ABI 3 callers. Native caller sanitizers instrument C/C++, not Rust. Historical selected-owner
  Miri used ABI 2; it is not a complete current ABI 3 gate. Wider platform/operational qualification remains later work.
- **Evidence:** [qualification report](core-qualification.json), [catalog witnesses](catalog-coverage.json),
  [SRE source and replay contract](README.md#required-sre-expression-sources).

### 28. Complete the approved integration stages through explicit contracts

| Stage | Decisions still needed |
| --- | --- |
| NIDL/dbengine adapter | Exact series/label mapping, name collisions, archive metadata, repeatable reads/concurrent expiry and histogram views. |
| Agent/Parent APIs | Query/internal interfaces, configuration, publication/error behavior and discovery semantics. |
| Go Cloud execution | Native/Cloud split, safe pushdown versus exact fallback, replica/source ownership, lossless fragments, permissions, cancellation and incomplete-result policy. |
| Independent rules | Rule-file contract, recording storage, result identity, templates, timers, persistence/reload and cross-role ownership/failover. |
| Netdata alert integration | State/events and notification integration alongside existing component health. |
| Platform and rollout | Broader OS/libc/device qualification, workload performance, adaptive limits and deployment policy. |

These are approved subsequent targets with unresolved design contracts. The standalone engine and this extraction
do not settle them. Cloud remains Go, the native engine remains Rust with C integration, and Alertmanager remains excluded.
