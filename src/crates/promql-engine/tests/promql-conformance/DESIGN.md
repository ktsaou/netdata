# PromQL engine: design decisions

The native query engine is a standalone Rust implementation of the pinned PromQL language with a typed C interface.
It evaluates raw storage through bounded pages and replay, supports streaming and owned results, and accounts for
resources with enforcement disabled. The containing Netdata tree supplies live dbengine/API integration;
the independent rule engine and Go Cloud execution remain subsequent stages.

This document extracts the approved design and its consequences. Its original core extraction records code checkpoint
`e5a9338d766eea359425641e1539760fca33c491` and qualification checkpoint
`5d1cb3c03caf202535161321ccde9f41b5803dc6`. Later source-bound qualification records preserve each actual implementation.

## Implementation status

| Status | Scope |
| --- | --- |
| Completed standalone core | Owned parser/type checker, float and native-histogram values, selectors, time/range/subquery evaluation, operators, joins, aggregations, stable and explicitly enabled optional functions, diagnostics, typed storage/output interfaces, C ABI 4, accounting and cooperative control. |
| Qualified evidence | Pinned upstream and required SRE expression panels, source-bound native/foreign checks, ownership/control regressions and actual C/C++ embedding. These finite checks do not prove equivalence for every possible input. |
| Implemented Netdata integration | Caller-owned contexts metadata, dual metric names, paged dbengine reads, fixed counter origin, endpoint/tier policy, Agent/Parent query/discovery APIs, fleet/explicit-host scope and bounded sealed responses. [Installation and API guide](../../../../../tests/promql-integration/GETTING_STARTED.md). |
| Approved later integration | Go Cloud distribution and a meta health engine parallel to existing health. |
| Designs still open | Distributed plan/transport, rule configuration/lifecycle/ownership, adaptive enforcement and broader platform/rollout policy. |
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
- **Status:** native expression core and Netdata API integration delivered; Cloud integration remains later work.

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
- **Evidence:** [comparative prototypes and performance study](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/tests/promql-prototype/README.md).

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
- **Effective argument edges:** keep the typed AST intact and use the existing preprocessing owner to resolve
  function arguments. Aggregation-parameter subtrees retain parentheses that upstream does not visit. Absence
  label inference inspects the resulting immediate node; it does not remove surviving parentheses a second time.
  The separate generic matrix-call path remains intact.
  See [absence argument qualification](absence-argument-qualification.json).
- **Evidence:** [owned frontend](README.md#owned-rust-frontend), [frontend source](../../src/frontend).

### 6. Own Go regex compatibility; reuse a Rust execution backend

- **Choice:** own Go-compatible syntax, byte/Unicode handling, captures, replacement expansion and selector
  optimizations. Use pinned `regex-automata` and `regex-syntax` as execution/encoding components.
- **Reason:** ordinary Rust regex or C++ RE2 behavior alone does not reproduce Go's ASCII `\w`, duplicate capture
  names or replacement-reference rules such as `$1x`. The backend does not define the accepted grammar.
- **Consequence:** this avoids writing a complete matcher from scratch while retaining responsibility for the
  compatibility frontend, generated Unicode data and dependency changes. Regex allocation estimates have the
  separate limitations described under decision 17 below.
- **Boolean execution:** selector fallback requests no capture outputs from the same PikeVM search and Input.
  Capture consumers retain their owned ranges. Shared search setup finishes capture export while the cache and
  estimate remain alive. Normalization/offsets, raw prefix checks and fresh execution-local caches remain.
  The [boolean-search panel](selector-boolean-qualification.json) measures output-buffer removal.
- **Backend construction:** a shared private compilation policy omits explicit capture markers only for selectors.
  Original parsing, frontend names, flat-program prefix/OnePass analysis and implicit group zero remain intact.
  Full capture consumers retain their backend groups. The
  [capture-table panel](selector-capture-tables-qualification.json) separates dependency estimates, query timings and
  fixture binary size; no mode is retained in each compiled matcher.
- **Shared byte validation:** validate bounded 1024-byte batches, repairing a crossing scalar against the original
  suffix. This removes per-rune accounting updates without a buffer, dependency or caller change. Malformed input
  charges through its first invalid byte; valid U+FFFD remains accepted. The initial control check, bounded later
  polls and existing terminal phase remain. The [UTF-8 panel](README.md#bounded-utf-8-validation) distinguishes
  unchanged work/memory from measured execution time.
- **Evidence:** [Go-compatible regex frontend](README.md#owned-go-compatible-regex-frontend),
  [pinned dependencies](../../Cargo.toml).

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
  [numerical source](../../src/numerical.rs).

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
- **Evidence:** [value model](../../src/value.rs),
  [owned values and output](../../src/result.rs).

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
- **Evidence:** [typed storage](../../src/storage.rs),
  [fold cache](../../src/range_cache.rs), [memory contract](contract.json).

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
- **Evidence:** [temporary interface](../../src/storage.rs),
  [paged float-bit owner](../../src/temporary_float_bits.rs).

The C page adapter retains a validated immutable pointer/count rather than an owned sample-descriptor array.
It checks the entire foreign shape and union footprint before cancellable publication, then decodes each requested
view with a lifetime tied to the wrapper borrow. Invalidation detaches the wrapper before cursor operations/close.
Only this immutable wrapper carries the public page's thread-marker requirements; the C owner remains unchanged.
Raw pointer/count avoids protected embedded slice references conflicting with provider cleanup during an owned
cursor move/drop. Generic Rust page paths and timestamp/error phases remain intact. See
[borrowed page qualification](README.md#borrowed-foreign-sample-pages).

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
  [certification owner](../../src/scheduled_expression.rs).

Final vector range output uses that certificate after every physical row and final label identity are checked.
It emits each row chronologically through one source cursor/page. Delayed cleanup and root negation preserve
first-productive-time series IDs with a bounded prefix pass and compact row-order metadata; collisions keep ordinary
evaluation. Direct clock initialization retains latched errors for their original lookup phase and withholds the
certificate when such an error exists. Preparation owns logical statistics and diagnostics; final traversal accounts
its actual work, resources and delivered points. See [final range output](README.md#certified-final-range-output).

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
  [paging driver](../../src/storage.rs).

## Runtime, ownership and accounting

### Compact group and join lookups

- **Choice:** accounted indexes store positions into the existing metadata arrays. Group identity remains hash-only;
  binary signatures and result-cache keys check cached hashes followed by complete encoded bytes. Arrays keep their
  original discovery/reduction order and first matching identity. Nonmonotonic ordinal rosters sort positions only.
- **Memory:** empty/singleton indexes allocate no slots. Sparse tables start at four slots, double at two-thirds
  load, and charge old/new capacity during replacement. Active join signatures switch to dense positions only when
  their domain fits the sparse allocation already required by the next insertion. Neither path stores raw values.
- **Phases:** both children finish before membership construction. Duplicate validation retains its original phase;
  reservations publish after successful values/labels and before false filtering. Cache first writers survive the
  whole logical scope. Aggregation releases its lookup before reversing or removing groups.
- **Tradeoff:** indexes reduce repeated searches and add metadata RAM. The measured AMD64 Frame remains 240 B and
  Active 16 B; the boxed binary Step grows 384→672 B. This does not remove computed-child replay or bound all metadata.
- **Evidence:** [lookup owner](../../src/position_index.rs),
  [measured grouping and joins](README.md#compact-group-and-join-lookups),
  [source-bound cohorts](core-qualification.json).

### 16. Observe resources now; keep every resource cap disabled

- **Choice:** account current/peak owned and query-held borrowed bytes, sample/work totals, retained series/groups,
  output/diagnostics, active/peak executions and elapsed time. Keep memory, sample/work, regex, cardinality, output,
  concurrency-admission and implicit engine-timeout enforcement disabled.
- **Reason:** one fixed profile would be excessive for small devices and restrictive for large Parents. The
  user explicitly replaced the earlier budget-enforcement proposal with accounting-only operation.
- **Consequence:** observations support later adaptive policy, but no adaptive formula, admission queue or fixed
  defaults are selected. Actual allocator OOM may remain process-fatal. Caller-requested cancellation/deadlines
  remain independent controls; representation/provider validity checks still apply.
- **Evidence:** [accounting](../../src/accounting.rs), [runtime contract](contract.json).

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
  [regex backend accounting](../../src/frontend/regex_nfa.rs).

### 18. One precisely accounted shared owner; explicit cancellation attribution

- **Choice:** use one local strong-only shared wrapper with known allocation layout and structural Box trait
  erasure. Count one physical allocation across aliases and release its charge after final payload/deallocation.
- **Reason:** payload-only accounting misses shared headers/ownership allocations. A uniform owner covers bounds,
  matchers, diagnostics, selections and other shared engine state.
- **Consequence:** reference counting/layout code requires focused qualification. This wrapper does not add Weak
  or copy-on-write support. Engine-created cancellation allocations remain charged to their creator, including
  cross-engine use; caller/default tokens remain external and query-held references are observed separately.
  Cycle-free accounting roots avoid leases retaining their own account indefinitely.
- **Evidence:** [shared owner](../../src/shared.rs),
  [control ownership](../../src/storage.rs), [accounting roots](../../src/accounting.rs).

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
  Explicit handles also accept `BorrowedView`: a shared cache or pool loan measures this query's exposure without
  attributing the underlying allocation to its owned workspace. Wrapper ownership remains charged to accounting.
- **Evidence:** [C allocation interface](../../include/netdata-promql.h),
  [provider bridge](../../src/c_abi/provider.rs),
  [output cleanup](../../src/c_abi/output.rs).

### 20. Typed cursors and one coherent C ABI 4

- **Choice:** expose opaque engine/query/result/report/control handles, counted byte views, tagged samples,
  complete typed histograms and explicit paired ownership. Providers may conservatively prefilter; Rust applies
  actual matcher semantics. Selection, series, cursor and query scopes close through their declared lifecycle.
- **Reason:** typed cursors avoid caller materialization of the complete query dataset and define callback lifetimes.
- **Consequence:** borrowing, successful NULL tokens and failed-open cleanup are explicit contracts. Headers,
  adapters, callers and observations use one coherent ABI 4; incompatible versions are rejected and ABI 2/3
  consumers must rebuild. Optional temporary callbacks are supplied as one complete group or absent until needed.
  Callers check ABI version before using unprefixed observation layouts.
- **Metadata discovery:** the additive `netdata_promql_discover` operation uses the same query accounting,
  provider lifetimes and provisional output while opening no sample cursors. It validates every supplied series
  before matching, releases each series promptly and closes provider scopes before final commit. Selector unions,
  final ordering and endpoint limits belong to the caller. Returning END from a label-only metadata row is
  successful early stop; ordinary expression-output status rules remain unchanged. No ABI4 table layout changes.
- **Caller preparation:** time, duration and limit parsing return owned reports. A synchronous report callback can
  borrow that report's actual account for preparation and error-body backing, without returning an accounting pointer.
  Allocation handles retain their creator account after the callback/report ends. Limit syntax, width and diagnostics
  follow pinned Go AMD64; an explicit requested limit remains caller result policy, with enforcement otherwise disabled.
- **Concurrency:** immutable compiled queries have independent execution contexts; cancellation may be concurrent.
  Access/release of a handle must not race. Getter views belong to their live result/report owner.
- **Evidence:** [C header](../../include/netdata-promql.h),
  [checked ABI](../../src/c_abi/checked.rs).

### 21. Synchronous provisional chunks plus an independently owned collector

- **Choice:** execute callbacks synchronously on the caller's thread. Borrowed chunks remain provisional until
  final commit; failure aborts output. Consumers can release chunks promptly. An accounted owned collector can
  retain the complete answer until its own release.
- **Reason:** output itself can exceed input working memory; streaming gives API/internal consumers a way to avoid
  retaining the whole answer while collection remains useful for callers needing independent ownership.
- **Consequence:** there is no post-return provider access, output spool or selected HTTP publication policy.
  Owned collection deliberately grows with the answer and does not inherit the raw-page memory guarantee.
- **Singleton floats:** the C adapter borrows one stack sample descriptor through its callback and error copying.
  Labels keep their existing copied descriptors; other point shapes retain pointer-stable general buffers.
  Callback cadence, both point-work/control phases and transactional publication remain unchanged. No persistent
  buffer, batching or additional retained point is introduced. See
  [singleton output qualification](README.md#borrowed-singleton-float-output) for allocation, timing and footprint costs.
- **Evidence:** [output model](../../src/result.rs),
  [C output bridge](../../src/c_abi/output.rs).

### 22. Contain ordinary Rust panics; preserve source failure semantics

- **Choice:** catch ordinary unwinding at the C boundary, discard the affected execution context and return an
  Internal error. Require panic-unwind builds, including the standalone `release-min` profile; install no process
  runtime/executor/hooks. The standalone crate uses Rust edition 2024 and MSRV 1.91 and builds independently
  of the Agent's other Rust components.
- **Reason:** a Rust engine bug should not unwind into a C caller or require a process-wide runtime change.
- **Consequence:** allocator abort, invalid foreign pointers, foreign unwinding/longjmp and double-panic abort are
  outside ordinary panic containment. Predictable reached upstream native faults retain their Runtime/WrappedRuntime
  semantics; they are not silently converted to arbitrary successful values or confused with Rust bugs.
- **Evidence:** [crate profiles](../../Cargo.toml),
  [unwind guard](../../src/lib.rs), [C containment](../../src/c_abi/mod.rs).

### 23. Cooperative control preserves failure and diagnostic precedence

- **Choice:** poll explicit cancellation/deadlines and record work through variable task, metadata, label-byte,
  parenthesis and diagnostic walks. A completed provider/output failure retains precedence over later cancellation.
  Successful/END callbacks poll before diagnostic conversion; position scans still charge work on failure paths.
- **Reason:** paging alone does not stop long metadata/preprocessing work, and cancellation must not replace an
  already completed source failure or publish an incomplete diagnostic.
- **Consequence:** formatting uses accounted candidate storage before publishing a complete message; successful
  output commit has no later fallible setup. Controls do not preempt a blocking foreign callback.
- **Evidence:** [control/work observations](README.md#cooperative-control-and-work-observations),
  [annotation owner](../../src/annotations.rs).

## Netdata data policies

### 24. Query the decoded retained Netdata dataset

- **Choice:** existing collectors, Prometheus profiles, NIDL entities and decoded retained values are authoritative.
  Evaluate complete PromQL over those inputs; do not add a separate original-exporter sample store.
- **Reason:** Netdata already collects and structures these metrics. The new language should use that existing data.
- **Consequence:** collector transformations, compression/downsampling and retained statistics remain meaningful.
  Language compatibility cannot reconstruct original values, gaps, timestamps or histogram meaning lost before evaluation.
- **Status/evidence:** approved production direction; the historical
  [dbengine replay prototype](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/tests/promql-prototype/README.md#implementations-and-input-contract) demonstrates a limited
  retained-data path. The completed typed core does not include the live NIDL/dbengine adapter.

### 25. Expose exact and underscore names as distinct logical series

- **Choice:** expose both exact context names and underscore forms, such as `system.cpu` and `system_cpu`, without
  a `netdata` prefix. They share stored samples but remain ordinary distinct query-visible `__name__` values.
- **Reason:** support Netdata's existing context vocabulary and conventional PromQL identifiers simultaneously.
- **Consequence:** selectors matching both names return both logical series; aggregates may count both and functions
  removing metric names may cause ordinary duplicate-label errors. There is no special alias deduplication. Dotted
  names use standard quoted/string-selector syntax, not a grammar extension.
- **Caller policy:** Netdata has selected `netdata_context` to distinguish colliding context aliases, separate
  host-label namespacing and a reserved `netdata_` prefix. Chart keys beginning with that prefix, and the chart
  key `__name__`, receive the once-only `netdata_chart_label_` escape. That adapter remains caller work.
  Future native-histogram storage/grouping remains a separate decision.
- **Evidence:** [NIDL model](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/docs/NIDL-Framework.md),
  [retained-data prototype contract](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/tests/promql-prototype/README.md#implementations-and-input-contract).

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
- **Caller policy:** Netdata has selected explicit API lookback overrides, frozen promised-read errors and parent
  fleet scope with explicit host prefixes. Their live implementation remains caller work. Independent rule
  configuration and future native-histogram storage remain separate decisions; frozen ownership is not
  transactional isolation.
- **Evidence:** [retained point model](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/src/libnetdata/storage-point.h),
  [storage cursor interface](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/src/database/storage-engine.h),
  [prototype point projection](https://github.com/ktsaou/netdata/blob/feature/promql-rust-core/tests/promql-prototype/README.md#implementations-and-input-contract).

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
- **Retained db210124 evidence:** 508 fixture units in each optimized standalone build and506 production units, finite
  expression/SRE panels and rebuilt ABI 4 C/C++ callers on three GNU targets. The earlier501/499-unit ABI 3
  statistics checkpoint remains historical; each cohort retains its actual source, binary and interface version.
  Native caller sanitizers instrument C/C++, not Rust. Historical selected-owner Miri used ABI 2; it is not a
  complete current ABI 4 gate. Wider platform/operational qualification remains later work.
- **Evidence:** [qualification report](core-qualification.json), [catalog witnesses](catalog-coverage.json),
  [SRE source and replay contract](README.md#required-sre-expression-sources).

### 28. Complete the approved integration stages through explicit contracts

| Stage | Decisions still needed |
| --- | --- |
| NIDL/dbengine adapter | Netdata has selected its contexts identity, frozen reads, counter-only storage and paged provider contracts. Their live implementation/qualification and any future native-histogram storage remain caller work. |
| Agent/Parent APIs | Netdata has selected query/discovery endpoints, feature configuration, host scope and sealed publication. Library protocol primitives are implemented; endpoint/transport acceptance remains caller work. |
| Go Cloud execution | Native/Cloud split, safe pushdown versus exact fallback, replica/source ownership, lossless fragments, permissions, cancellation and incomplete-result policy. |
| Independent rules | Rule-file contract, recording storage, result identity, templates, timers, persistence/reload and cross-role ownership/failover. |
| Netdata alert integration | State/events and notification integration alongside existing component health. |
| Platform and rollout | Broader OS/libc/device qualification, workload performance, adaptive limits and deployment policy. |

The generic core does not own those caller integrations. Cloud remains Go, the native engine remains Rust with C
integration, and Alertmanager remains excluded.

## Precise requests and protocol primitives

- **Choice:** preserve normalized seconds/nanoseconds and raw duration nanoseconds at the request boundary.
  Keep the existing millisecond sample/evaluation grid, including the pinned Go clock projections and reached
  errors. Millisecond request entry points remain available; precise Rust/C entry points share ABI 4.
- **Ownership:** protocol parse reports and selector handles own their data after the source bytes or engine are
  released. Matcher/getter views remain borrowed from their handle. Label conjunctions match caller-owned bytes
  with explicit work accounting and cancellation checks. The core owns no URL router, HTTP client or catalog API.
- **Catalog behavior:** evaluate frozen selector matchers before copying a provisional descriptor. Continue
  draining nonmatches for reached errors/annotations; preserve upstream's empty-expansion behavior. Narrow
  selectors therefore avoid retaining the unrelated catalog's labels.
- **Evidence:** `protocol-original.json` and `precision-original.json` preserve unchanged original outcomes.
  Their replay tools bind each actual binary/source cohort. These checks qualify primitives and execution clocks;
  they do not qualify Netdata endpoints, forms, metadata discovery or retained-data projections.

## Basic query statistics

- **Choice:** expose the six pinned timing fields and three sample fields through execution observations and
  ABI4. Timings measure actual Rust execution phases. Evaluation includes preparation and inner evaluation;
  execution includes evaluation and result ordering. Queue time is zero while admission remains disabled.
  Observations are finalized before the output commit callback. Per-step arrays and their optional feature remain off.
- **Logical participation:** `totalQueryableSamples` counts source samples participating in the primary logical
  selector/windows, including overlaps and consumed subquery windows. Preparation carries compact counts through
  completed folds. Parent, ordering and output replays do not credit the same logical work again. The implicit
  `info()` selector participates before base-series or winning-info filtering.
  Failed or cancelled queries retain reached primary participation. A completed parent replaces its already
  counted children. Subquery bodies and invariant inner evaluations have private counts; successful scopes
  publish the merged windows or reached outer copies. Partial instant admissions and completed source windows,
  matrix series and info rows remain observable without retaining their raw history.
- **Actual reads:** `samplesRead` counts useful, nonstale source samples admitted by real consumers. Repeated
  numerical reads count again; acquisition-only validation, unused page prefixes/lookahead and generated subquery
  rows add no source reads. Fetched page entries and operator work remain separate counters.
- **Owned peak:** `peakSamples` measures simultaneously owned initialized sample values, scalar slots and float
  work buffers. A float is one unit. A native histogram uses the pinned Go AMD64 weight
  `floor((176 + 8 * combined_slice_lengths) / 16)` for its admitted five-slice representation. The timestamp
  selector has its upstream one-unit source exception. Owned copies count independently; shared aliases, rank/index
  metadata, numerical accumulators and borrowed provider pages add no owned samples.
- **Consequence:** peak values describe Rust's bounded owning lifetimes, which differ from Go's materialized
  matrices. Moves transfer leases; replacements and completed outputs retain their real overlaps. Cancellation,
  errors and unwinding release owners. A convenience collector retains its output samples until that result is
  released; the execution-time peak includes those outputs. These statistics are neither RAM-byte measurements
  nor enabled query limits. No raw history is materialized to imitate upstream's memory use.
- **Evidence:** `src/statistics.rs`, `src/accounting.rs` and focused tests in the scalar/window, histogram,
  numeric, temporal, classic and temporary-backing owners. Fresh cohort receipts bind the tested sources and callers.

## One coherent interface version

- **Choice:** ABI 4 replaces ABI 3. The statistics fields enlarge the observation structure, so every existing
  caller must rebuild and check the library version before using tables or observation outputs. Versioned inputs
  reject older tables before dispatch. There is no parallel legacy observation interface.
- **Reason:** an old ABI 3 declaration is 584 bytes while the enlarged structure is 704 bytes on the qualified
  AMD64 target. Retaining version 3 would admit callers whose observation destination is too small.
- **Evidence:** `src/c_abi/types.rs`, `include/netdata-promql.h` and the previous-header rejection caller.
  Historical ABI 2/3 qualification remains attributed to its original sources, layouts and binaries.

## Backed matrix ordering

- **Choice:** expose the existing pinned Go index sorter to synchronous C callbacks, using their actual query
  account and control. The caller compares immutable backed raw-label keys while Rust exclusively permutes a
  compact caller-owned index array. No complete result timeline is required.
- **Reason:** stable queries can produce distinct rows with identical displayed labels after grouping/projection.
  A stable secondary series-ID order or an arbitrary sort changes observable equal-label provenance.
- **Consequence:** comparisons return fallible statuses; errors own their bytes in a child report and win over
  cancellation raised by that callback. Failed sorting may leave a partial permutation, discarded by the enclosing
  output transaction. The helper observes borrowed exposure and actual sort work without copying the index array.
- **Evidence:** `src/c_abi/sort.rs`, `src/go_sort.rs`, the empty-group-label execution/collector fixtures and the
  six actual C controls. The new callback is additive; existing ABI4 table layouts remain unchanged.

## Retained library qualification

- Both optimized builds pass522 fixture units; the production feature set passes520 units. Strict lint,
  formatting and the installed Rust1.91.0 minimum-version checks pass.
- The retained full expression cohort at signed db210124 executes290956 cases, with overlapping families kept separate. All231540 required
  SRE executions pass across both builds and pages1/17/1024, including candidate-owned recording dependencies.
  Neither panel has an unresolved expression or bitwise difference. The subsequent isolated metadata/API additions
  do not change arithmetic owners; these historical panels retain their actual source and binary identities.
- At f72d6aa, the precise-clock panel passes27 comparisons per build. Protocol primitives pass230 cases and528 label
  conjunctions per build; the separate pinned limit panel passes34 cases per build. HTTP wrapper cases remain Netdata integration work.
- Actual rebuilt AMD64, ARM64 and ARM32 C/C++ callers pass448 records:432 current-caller records and16
  previous-header rejection records. Each layout has58 structures,17 callbacks and33 shared constants.
- Metadata discovery checks zero sample cursors, prompt series release, normal early stop, provider/output failures,
  cancellation and allocation handles retained beyond reports. Report-account callbacks and64-bit limits are exercised
  by actual C callers, including ARM32. Existing ABI4 table layouts are unchanged.
- Discovery selector preparation polls cancellation and counts work inside its execution timing. A prior-cancelled
  roster performs no selector copy; all six metadata C cases pass in that cohort. The current
  [descriptor-capacity cohort](label-descriptor-capacity-qualification.json) adds a seventh nine-label case. The corrected loop's fresh
  units/caller cohort remains distinct from the unchanged protocol/clock/arithmetic cohorts above.
- The current index-sort bridge adds four focused units and six controls per C caller. The prior discovery517/515
  and430-record cohort remains attributed to d56f263; existing arithmetic and parser owners are unchanged.
- The additive native quoting getter shares the existing Unicode17 printability table and checks its output shape.
  One new unit and twelve rune boundaries per current C caller qualify it; table contents and quoting algorithms
  are unchanged. The prior sorting521/519 and448-record cohort retains its56fea9c source identity.
- Native Clang C ASan/UBSan checks and the rejected panic-abort compilation pass. Caller instrumentation
  does not instrument the linked Rust archive. No current full-engine Miri gate is claimed.
- `core-qualification.json` binds every fresh receipt to its actual sources and binaries. These finite checks
  qualify the library; live metadata, dbengine projection, HTTP routing and response delivery belong to Netdata.

## Final range output qualification

- The finalized 142-file source cohort passes 527 fixture units in each optimized profile, formatting, strict lint
  and the installed Rust 1.91.0 check. Final ARM64/ARM32 checks compile that source. Earlier actual foreign C/C++
  runs retain their pre-lint source identities; fresh final C/C++ execution is native AMD64.
- Both builds and pages 1/17/1024 pass 128,652 builtin comparisons and 231,540 required SRE comparisons, with no
  expression mismatch or paired-bit difference. Recording chains consume candidate-owned completed results.
  Focused schedule, default-profile, precise-clock and cursor-policy panels retain their separate complete outcomes.
- Page 1 matches all 82 unchanged cursor originals. Pages 17/1024 match 74 originals and separately qualify eight
  approved physical-Seek outcomes against independent Go controls; their raw original disagreements remain evidence.
- Current C/C++ callers preserve ABI4 layouts and reject ABI3, and the panic-abort build remains rejected.
  A controlled Netdata build passes 34 HTTP checks and 24 contexts-readiness scenarios across array/Gorilla storage.
  Both fixture processes exit successfully. No production service is part of this qualification.
- A 13-case before/after panel preserves complete results. Its 128-point selector fetches 256 entries instead of
  9,372 at page size 17, with the same 6,961-byte workspace peak. Generated final-output checks stream up to
  100,000 points through one active cursor with history-independent raw workspace. These observations do not
  establish DBengine disk throughput or whole-process memory.
- [core-qualification.json](core-qualification.json) records the final source/binary cohort and retained evidence
  identities. The public language, numerical, provider, C ABI and HTTP contracts are unchanged.

## Completed unique float join scans

- **Choice:** certify unambiguous bare root joins between two raw float selectors. The original first replay
  completes discovery, arithmetic, reservations and duplicate checks for the current timestamp. After successful
  completion and diagnostics, later output rows reuse its emitted-ordinal bitmap and read their own operand pair.
- **Ownership:** query-local bindings map frozen cache ordinals to original side positions. One accounted snapshot
  identifies all effective clock/options, step index, exact lookback and data bounds. Replacement releases the prior
  snapshot first; errors and partial replays never publish one. No raw values or timeline are retained.
- **Compatibility:** exact frozen keys and first-writer labels remain authoritative. Native operand values, filled,
  ambiguous, delayed-name, invariant and unsupported clock shapes retain ordinary replay. Initial validation and
  error priority remain complete; errors tied to omitted physical operations follow the existing provider policy.
- **Grouped ranges:** existing direct/rate/earned Float operand proofs can admit `group_left`/`group_right`.
  Keep unique full float rows on both sides and unique matching signatures on the one side. Enumerate every
  many-side row; reject ambiguous histories, missing output bindings and conflicting cache ordinals. Pair positions
  remain in original left/right order while frozen keys use many/one label order. Existing cold validation,
  successful Done, exact-clock membership and fresh selected reads remain the same owners; no state field is added.
- **Grouped evidence:** [qualification](grouped-pair-replay-qualification.json) records all comparisons and costs.
  Large-case reads/allocation traffic decrease with unchanged peaks; one mapped case adds 81 B peak and rejected
  native/mixed/history proofs add up to 39 work units and 112 cumulative bytes. Timings retain slower tiny controls.
- **Evidence:** `src/vector_matching.rs`, `src/evaluator_roster.rs`, `src/evaluator.rs`, `src/execution.rs`, the
  changing-membership/history fixtures and actual C caller's first-output and selected-pair failure cases.

## Completed instant numeric folds

- **Choice:** a productive true-instant root Numeric call over a raw matrix selector may retain its completed
  preparation folds when source and scope headers equal the effective evaluator header. Other clear sites and
  non-instant/composite/subquery/Rate/Fold paths keep their existing behavior.
- **Ownership:** stable in-place compaction keeps only that root/selector's completed float-or-absent snapshots.
  The existing sorted records and exact request key remain authoritative; no new array, raw page or history is
  retained. Absent summaries are useful because final ordinary traversal still visits their physical series.
- **Compatibility:** final initialization, scalar parameters, labels and diagnostic/statistics handling remain
  unchanged. Exact-key misses recompute normally. Parameterless functions have no separate outer initialization;
  omitted internal fold reads follow the existing physical-operation policy. Completed backing closes before reuse.
- **Evidence:** `src/execution.rs`, `src/range_cache.rs`, exact quantile/absent/history controls and the actual C
  caller's error armed after first output. The public instant step0 is distinct from effective evaluator step1.

## Completed direct rate join scans

- **Choice:** the original direct-rate cohort admitted `rate(raw_selector[range])` to the float join certificate.
  The current [operand proof](#completed-mapped-rate-join-discovery) also covers pure wrappers and the complete
  existing rate-function family and [per-series window folds](#completed-per-series-fold-join-discovery).
  Producer, source and scope grids must match the effective evaluator grid.
  Fixed, invariant, anchored, smoothed and unproven computed shapes retain ordinary replay.
- **Clocks:** certification checks the nanosecond range-plus-offset sum and both physical/extrapolation window
  endpoints before reuse. Physical selection and extrapolation truncate different expressions; fractional inputs
  retain their full existing arithmetic. A failed eligibility check uses ordinary evaluation without a new error.
- **Ownership:** range outputs preserve physical selector ordinals. Existing side positions bind those ordinals
  to frozen join identities; the existing accounted one-step membership bitmap and selected-pair evaluator suffice.
  No additional label roster, value array, raw history, cache or provider requirement is introduced.
- **Compatibility:** complete first-pass discovery, matching, diagnostics and successful completion still precede
  publication. Nonfloat values discard the candidate. Later selected-pair reads retain their existing errors,
  statistics and exact-key authority; omitted operations follow the established provider policy.
- **Evidence:** `src/execution.rs`, complete reversed-order/membership/comparison/mixed-operand fixtures and actual
  C first-pass provider-error, first-output cancellation/error and release controls. Matched work falls while raw
  reads and tracked workspace peaks remain unchanged in the recorded generated workload.
- **Phase proof:** the C error case uses the normal run's cursor observed before first output and checks a
  later physical series. Native traces establish final evaluation; output begin alone precedes preparation.

## Borrowed binary scheduling proof

- **Choice:** resolve the existing raw/rate selector nodes and call the shared pure schedule predicate directly.
  Keep the prepared effective lookback, timestamp-selector marker, grid and left-before-right short circuit.
  The temporary evaluator previously used only for this proof performed no evaluation, work charge, control poll
  or provider callback. Evaluators that produce results remain in place.
- **Scope:** all binary eligibility and certification rules remain authoritative, including rejected schedules and
  later nonfloat rejection. This adds no runtime state, cache, sample buffer, feature, dependency or public contract.
- **Memory:** the finite native comparison saves 960 acquired/released workspace bytes in 540 query/profile cases;
  480 cases retain their allocation counts. Complete outputs, reads, work and cleanup match. Measured whole-query
  workspace peaks remain unchanged; allocation traffic is distinct from peak RAM and RSS.
- **Evidence:** [binary scheduling qualification](binary-schedule-qualification.json) binds the accepted/current
  source and executables, complete raw/rate/mixed/fallback/native/precise controls, native units/C callers and timing.
  Earlier language, SRE, ARM and Agent cohorts retain their actual historical source identities.

## Completed direct float-map join scans

- **Choice:** resolve the existing earned scheduled-selector marker for the operand. Its existing grammar proves
  one prepared vector path, pure scalar side arguments and projected-label uniqueness. Direct and composed Float
  maps, parameterized round/clamps, explicit-vector calendars, vector/pure-scalar operations, parens/unary, label
  projections and supported timestamp or histogram count/sum/avg conversions can qualify as outer operands.
  Require an actual raw Selector and
  identical outer/leaf producer/source/scope grids. Preserve all root, matching, uniqueness, selector-schedule,
  exact-key and successful-Done gates. The separate direct rate-family branch keeps its actual-arity-one restriction.
- **Ownership:** reuse existing pair bindings and accounted current-scope membership. Later outputs read selected
  wrapper operands afresh through canonical-to-physical mapping. First-final delivery drains all pairs before
  publishing membership. Release the previous completion before replacement. No raw history array, purity walk,
  function whitelist, additional runtime state or cache is introduced.
- **Parameters and omission:** normal evaluation computes scalar arguments before reading the vector, including
  either scalar position in a mixed Binary expression. Full clamp
  can emit an empty scope; completed empty and populated membership remains exact-key-specific. NaN bounds produce
  present float NaNs, and native values retain normal filtering. Sparse/stale and reordered inputs preserve wrapper
  ordinals. Source-reading scalar parameters, zero-argument clocks, unproven shapes and unsupported schedules retain
  ordinary evaluation. Direct timestamp selectors can lack the leaf producer required by the existing grid check;
  timestamp of a computed vector may qualify and observes that vector's source-time policy. Scalar parameter clocks
  may intentionally be invariant. No numerical/default/ABI/storage/reference contract changes.
- **Failure phases:** separately calibrate preparation and actual first/fourth/completed output acquisitions.
  Preserve first-empty, next-scope, cold pair suffix, warm selected and last-empty provider failures and provisional
  abort. Actual surviving private frames establish parameter-phase interruption before vector acquisition; a work
  budget alone is not phase evidence.
- **Tradeoff:** repeated reads, work and allocation traffic fall in qualifying ranges. All 384 scaling peaks in
  each retained one-argument/parameter cohort remain unchanged. Existing reuse owners raise some small-case peaks:
  82–312 B in the earlier one-argument cohort and 146–312 B in the parameter cohort. Some mixed-kind rejected
  certificates add 32 cumulative bytes and 16 work units with unchanged peaks. These are measured finite costs,
  not a global peak-memory reduction. Accounted bytes are not RSS; caps remain disabled.
- **Composed checkpoint:** [composed-map qualification](composed-map-join-qualification.json) binds complete native/compiled-ARM
  outcomes and nonuniform label/time/native expectations. Pair selection keeps outer ordinals; existing source maps
  translate through intermediate projections. No second purity/clock walk or raw-ordinal substitution is added.
  All 240 scaling workspace peaks and 2,006 instant observation groups match; 396 small groups add 312 B at peak
  (at most 1.10%) through existing completion metadata, while 1,524 groups reduce reads/work/allocation and none
  increase them. These are
  measured finite costs, not a universal peak-memory reduction.
- **Direct operands:** [earned-operand qualification](earned-operand-join-qualification.json) extends that checkpoint's
  outer Float restriction to every already-marked class, preserving the raw Selector and separate rate paths.
  Pair certification still requires actual unique Float operands/output; native-preserving results remain ordinary.
  Bare label projections keep child encounter order, which can differ from the sorted final root order. Cold fault
  landmarks therefore follow actual pair order; warm selected reads retain the requested outer ordinal.
  The finite panel passes 1,986 complete native and 1,196 compiled ARM matches, with independent value/label/time
  expectations. All 966 instant observations and 240 scaling workspace peaks match. In 774 groups, reads, work and
  cumulative workspace acquisition decrease. Forty-two small groups add 168 or 312 B at peak (at most 1.40%);
  six native-preserving fallbacks add 15 work units and 32 cumulative workspace bytes with unchanged reads/peaks.
  Fixed paired timings retain all five slower controls. No new state, cache, raw arrays or public contract is added.
- **Evidence:** [parameter-map qualification](parameter-map-join-qualification.json) binds the earlier parameter,
  changing-membership, interruption, complete native/compiled-ARM, actual C/C++ and timing panels, including every
  slower group and distinct development failure. The [one-argument cohort](float-map-join-qualification.json) and
  [initial abs cohort](abs-join-qualification.json) retain their actual source/binary identities.

## Compact one-to-one result labels

- **Choice:** one-to-one projection with no included fields copies only retained labels. It removes empty values,
  conditionally removes the three reserved metadata names, then applies the existing `on`/`ignoring` predicate.
  Grouped and nonempty-include projections keep their existing replacement/removal path.
- **Ownership:** the existing accounted label vector grows with output fields rather than discarded input fields.
  A shared private field-copy helper preserves exact byte capacities and checked copy checkpoints. Public label
  types, complete pair keys, first-writer cache identities and independently owned output copies are unchanged.
- **Compatibility:** virtual reset precedes the original reserved-name preflight. A surviving empty name still
  reports its wrapped runtime error before matching can remove it. Existing reserved-removal callers share that
  same authority. Source order, cancellation/deadlines and partial-owner cleanup remain checked.
- **Boundaries:** contexts/catalog/provider metadata ownership and raw paging are unchanged. This targets retained
  projection capacity; input metadata, cache keys and collected output remain separate cardinality-dependent costs.
- **Evidence:** projection memory/error/cancellation controls, complete matching/include cases, actual derived
  empty-name expressions, discard-output statistics and source-bound whole-query comparisons.

## Single-series range head preparation

- **Choice:** the direct raw range-function branch skips step-kind collection and the accumulated-descriptor
  head sweep. Each physical series produces at most one optional value per tick; zero/one values of one kind
  cannot change a head summary. Generic multi-output preparation retains its existing bookkeeping.
- **Authority:** evaluation, parameters, initialization, label hashing, descriptor kinds/order, folds, diagnostics,
  clocks, statistics and roster publication keep their original owners and order. Kind changes across different
  ticks remain recorded independently. Meaningful work and final execution still poll cancellation/deadlines.
- **Ownership:** no cache, index, raw window, value array or public type is added. The removed one-entry scratch
  vector accounts for a small constant workspace reduction; raw paging is unchanged.
- **Boundaries:** other uniqueness and final replay costs remain. Removing this quadratic no-op scan does not
  make total preparation linear or reduce raw reads. Matched query timings qualify generated workloads only.
- **Evidence:** a complete-result/statistics/release work regression, retained native/mixed/generic/error/clock
  controls, and both-profile/page source-bound comparison and integration cohorts.

## Range preparation label uniqueness

- **Choice:** ordinary range-function preparation indexes descriptor positions with the existing accounted
  `PositionIndex` instead of scanning every earlier hash. The index stays local to the original duplicate-check
  phase and is discarded before roster sorting/publication.
- **Authority:** recompute each current label hash in its original order; compare only full hashes against
  immutable prior descriptor hashes. Unequal labels with equal hashes remain duplicates. Complete preparation,
  including later provider errors, still precedes duplicate detection.
- **Ownership:** release the finished collection index before allocating uniqueness metadata. Neither index
  owns raw samples or provider callbacks. The new table alone can exceed the old hash vector's size; whole-query
  memory benefit must come from measured lifetimes, not a smaller-table assumption.
- **Controls:** explicitly check cancellation after lookup before reporting duplicates or inserting a position,
  and after insertion. Rebuilds retain existing checked allocation/probing and cleanup.
- **Boundaries:** kind-aware streaming checks, delayed cleanup, generic merges and metadata-preserving functions
  keep their existing behavior. Hash collisions can still cause repeated probes; this is no worst-case linearity
  guarantee. Raw reads, numerical evaluation and public interfaces are unchanged.
- **Evidence:** [range uniqueness qualification](range-uniqueness-qualification.json) records complete work,
  collision/error/control checks and both-profile/page comparisons, alongside current caller and Agent cohorts.

## Validated average acquisition reuse

- **Choice:** the initial ordinary raw-matrix average accumulates provisionally during acquisition. A successful
  float-only acquisition can omit its separate consuming traversal. Bounds must fit the current non-wrapping
  window; statistics projection, buffered windows and anchored reductions keep their original path.
- **Authority:** the original ordered average accumulator preserves compensation, overflow switching, NaN bits,
  signed zero and stale handling. Complete acquisition, page validation and the final control check precede read
  credit, finalization and publication. Generic reader callers still validate before invoking their helpers.
- **Histograms:** allocation state includes stale histograms. Populated native/mixed acquisitions retain consuming
  validation and its saved progress, with no repeated float accumulation. Later native arithmetic uses the original
  acquisition-then-consumption path, preserving failures and diagnostics.
- **Ownership:** reuse adds no raw samples, value array, backing store, cache or public interface. The existing
  page, stack accumulator and saved traversal state suffice. Callback errors remain tied to actual operations.
- **Boundaries:** wider private bounds can expose earlier-window values during acquisition; they retain ordinary
  consumption. This optimization targets physical reads and leaves other preparation and replay costs in place.
- **Evidence:** [average reuse qualification](average-read-reuse-qualification.json) binds complete results and
  credits, saved-state comparisons, calibrated failures, C execution phases, memory and matched query measurements.

## Validated float rate acquisition reuse

- **Choice:** the initial ordinary `rate`, `increase` and `delta` float pass collects its existing endpoints/count
  during acquisition. After complete validation and terminal control, a pure-float acquisition can omit its
  separate consuming traversal. One private `FloatStep::scan` serves both float-rate callers; Avg and rates share
  the unchanged current-window/bounds certificate.
- **Arithmetic:** last-minus-first still precedes every chronological reset addition. Reset replay, raw/no-matrix
  calls, instant/extended rates, statistics projection, buffered/wrapping/wider windows and fresh evaluation/cache
  boundaries retain their existing paths.
- **Histograms:** allocation state includes stale histograms and earlier reached history. For populated histogram
  acquisitions, discard provisional float state and consume normally with the completed progress before later
  native/mixed handling. Generic helpers still run after acquisition validation.
- **Ownership:** existing fixed endpoint fields and their sample leases suffice; no raw array, value cache,
  persistent layout or interface is added. Failed acquisition may report two earlier local owned endpoint units,
  while useful reads stay zero. That reflects ownership timing, not extra heap allocation.
- **Cost:** fewer reads and allocation/view traffic do not imply lower peak RAM. The actual fixture text increases
  and all slower controls remain explicit; source-bound evidence is in the
  [rate acquisition qualification](rate-acquisition-qualification.json).

## Histogram constructor key lookup

- **Choice:** a transient position index replaces repeated constructor lookup for larger classic histogram group
  sets. Up to two groups retain direct comparisons; third-group promotion uses their first-encounter positions.
- **Authority:** complete encoded label bytes define identity. Classic keys omit `le`; native correspondence uses
  complete labels after all classic groups are known. Hash collisions require byte comparison. Reset output labels
  remove empty values and cannot replace the original keys.
- **Ownership:** the index stores positions into existing keys and retires before plan publication. It adds no
  persistent plan field, key copy, raw value or history array. Whole-query peaks must be measured across lifetimes.
- **Controls:** checked lookup, promotion and replacement retain accounting and interruption checks. Explicit polls
  follow lookup/insertion and precede successful constructor publication.
- **Boundaries:** parsing, first group IDs, label preparation, separate passes, native/mixed semantics and later
  numerical/replay owners retain their behavior. Tiny-set lookup avoids unnecessary full hashing; larger collisions
  can still produce repeated probes.
- **Evidence:** [histogram key qualification](histogram-key-qualification.json) records complete identity, parsing,
  cancellation and memory controls, current caller/Agent panels and matched measurements.

## Histogram active-group distribution

- **Choice:** temporary group-ID lookups replace repeated active-group discovery and native correspondence.
  Direct lookup through four groups avoids tiny-set tables. Discovery counts bucket references in existing ranges.
- **Order:** release discovery positions before label-based enumeration, then index surviving sorted groups.
  Checked prefix counts reserve the existing bounds buffer; source-order scatter preserves every bound bit and
  physical row ordinal before the pinned numerical sort. Skipped groups keep zero-length ranges at their offsets.
- **Native handling:** the first corresponding native suppresses an unskipped classic group and that native;
  later natives remain unchanged. Live-cache updates and annotations keep their existing phases.
- **Ownership:** use existing range fields for counts and write cursors, restore them before replay, and release
  both indexes before choosing values. No new persistent field or raw-value/history array. Small surviving sets
  retain ordinary distribution.
- **Controls:** discovery, promotion, prefix/initialization, scatter and restoration remain checked/accounted;
  interrupted partial state is discarded before completion.
- **Boundaries:** complete vector collection and later numerical/binding/derived replay remain. A deeper nested
  OR/projection range exceeded the live fixture client's timeout in both compared Debug Agents at that source;
  the independent historical throughput witness is retained separately.
- **Evidence:** [active distribution qualification](histogram-distribution-qualification.json) binds exact
  pre-sort sequences, sparse/skipped/native/control cases, complete queries, caller/Agent checks and measurements.

## Ordered histogram streaming

- **Choice:** a histogram whose frozen binding order already equals canonical output order keeps one existing Step
  behind the descriptor adapter. This removes repeated full-vector construction per output row without a new cache.
- **Admission:** Quantile/Fraction scalar arguments must resolve to numeric literals. Require nonempty, complete
  one-to-one identity sources, matching full labels/hash/drop flags and single Float output heads. Invariant and
  boundary owners retain their ordinary route. Runtime admission requires `All`; other filters keep selected replay.
- **Publication:** certify after the histogram's own complete node finishing. Its own logical preparation starts
  with a new false Plan and is explicitly excluded at runtime. Ancestors may consume the finished child; stored
  child diagnostics and primary sample-unit owners remain authoritative. No discovery-completion credit is added.
- **Traversal:** each invocation still performs fresh full-vector discovery, ordinary Count/Native reads and actual
  child Done. Keep descriptor hash/head/drop normalization. Align sparse samples by their emitted canonical ordinal;
  an empty current tick drains normally. Vector forwarding retains the shared child, while ancestor truncation drops it.
- **Fallbacks:** dynamic scalar arguments, Quantiles, aliases/reordering, empty rosters and One/Ordinals/Contributors
  retain ordinary behavior, including no-match early exit. Numeric and physical bound order remain unchanged.
- **Emission:** outside own preparation, Quantile/Fraction reuse the binding ordinal already selected by `choose`.
  Checked subtraction/get and key/occurrence validation retain the missing-binding error. Dynamic and selected
  routes use the same identity; logical preparation and Quantiles retain their ordinary searches.
- **Duplicate proof:** canonical assembly merges equal hashes before certification. The complete singleton binding
  bijection therefore proves distinct hashes, allowing certified nonlogical emission to omit its seen buffer.
  Uncertified, plural and own-preparation routes keep hash-only duplicate handling. Poll after checked label copy
  before phase publication or fallback sample ownership, even when the hash walk is omitted.
- **Costs:** one Plan flag and a longer-lived existing Step, with no additional index or cache. Presence/group and
  other metadata searches remain. Measure peak, allocation traffic and useful/physical sample counts separately; no
  universal linear-work or lower-RSS claim. Procedure and finite evidence belong to
  [the owning guide](README.md#ordered-histogram-streaming).

## Completed set-input discovery

- **Choice:** eligible ordinary forward instant and range sets reuse active-input membership after both original child
  discovery streams exhaust. Owning preparation and nested ordered bodies share this private Plan owner.
- **Authority:** a cold pass retains full label/ordinal resolution and requires float, strictly ascending unique
  side-row positions. The bitmap includes suppressed inputs, not only returned outputs. Matching-signature
  duplicates remain legal; existing complete-byte signature IDs remain authoritative.
- **Eligibility:** raw nonextended selectors, transparent parentheses, prepared label projections and already
  qualified sets must share compatible resolved source/scope/producer grids. Original immediate-child aliases are
  checked before projections strip source ranges or move descriptors; actual owned side rows are checked afterward.
  Native/mixed, multiple, aliased, invariant, copied, wrapping and unsupported shapes use the original path.
- **Phases:** publication at Right-to-Begin certifies complete discovery, not successful binary emission. Warm
  reconstruction retains original side order, Begin, choice, selected/unverified rereads, duplicate/diagnostic
  completion and positive Vector truncation. Completed prefixes can omit the nonselected acquisitions described
  below; the original discovery-only cohort retained every reread. Pair replay still publishes only after Done.
- **Reconstruction:** warm discovery enumerates the existing membership words and their lowest set bits in ascending
  physical position. The actual left-side length maps positions across sides; final padding is ignored. Work and
  controls cover every word, including zero words, and every active row, followed by the existing final poll.
  No additional buffer or field is retained. Sparse reconstruction avoids inactive descriptor visits; dense
  reconstruction adds one work unit per bitmap word.
- **Final evaluation:** the successful end of roster preparation clears only set discovery snapshots. Both ordinary
  and standalone derived-matrix consumers therefore perform a complete first final discovery; eligibility and Pair
  state survive. Failed preparation drops its local state.
- **Ownership:** one exact existing ReplayKey and dense bitmap per eligible Plan, with accounted owners and old
  state released before replacement. No raw values, labels, pair mappings or history are cached. The replay enum
  occupies the existing vector niche, adding no persistent Plan/Step field or Replay size.
- **Controls:** allocation, capture, reconstruction, replacement, publication and clearing retain polling and work
  accounting. Kind/order ambiguity discards capture while continuing the original cold streams.
- **Limits:** selected/unverified acquisitions and histogram/projection replay remain independent costs. Reduced raw reads can
  add a small current-scope metadata allocation; whole-query workspace and timing require actual measurement.

## Completed set-operand prefixes

- **Choice:** one scalar frontier records the greatest successfully completed kept set ordinal at the current exact
  discovery scope. A targeted body in its original set-ordinal domain omits acquisition, value handling and label copying for verified
  nonselected operands. The requested operand is always acquired freshly; All bodies retain every value.
- **Authority:** discovery alone grants no prefix credit. Fresh acquisition, value presence, label copying, full
  hashing, seen bookkeeping and the terminal control check must succeed before progress advances. Choices visit
  kept rows in ascending flat left/right order; excluded matching rows need no operand credit.
- **Phases:** skipped operands reuse the immutable descriptor's canonical full hash and reconstruct the original
  seen and deferred-duplicate state. Raw construction and every label projection create that hash with the exact
  owned labels; copies and merges preserve the pair. Successful fresh emission validates those bytes before
  granting credit. This does not generalize to grouping hashes or arbitrary descriptors. Unverified prefixes/suffixes,
  selected errors, original Vector truncation and Finished checks retain their paths. No extra draining manufactures completion.
- **Scope:** initialization and advancement share one owner/filter predicate. Matrix bodies with bypassed ordering
  and the actual Matrix preparation owner retain their existing activation. A certified singleton set body may
  also activate directly in either mode when no ordered roster exists; its filter then addresses emitted set
  ordinals. A present ordering layer retains its original translation. Unfiltered Vector bodies and Pair activation
  remain unchanged. An inactive context cannot
  read or update progress. An eligible All context seeds success without skipping. Every skip/update checks the
  actual effective frame key; replacement, new discovery publication and preparation clearing reset progress.
  Ordinary Done without a pending owner preserves successfully completed All-body progress.
- **Ownership:** the scalar fits the existing replay enum payload on qualified native and ARM targets. A synchronous
  borrowed key/target context adds no persistent Step field, owned buffer, values, labels or history. Existing Pair
  layout/accounting and current-scope bitmap ownership remain unchanged.
- **Serial callers:** this representation relies on current production scheduling. Certified children are proper
  raw/projection/set descendants; parenthesis preprocessing does not redirect to ancestors. An outer OrderedRows
  frame is not a Binary Step and finishes or truncates its child before the next one. Preparation/final Machines
  run sequentially and reentrant queries own fresh Rosters. Key identity alone would not protect future overlapping
  same-Plan A→B→A Steps; shared/interleaved Rosters or ancestor redirects must revisit the certificate.
- **Controls:** fresh publication and skipped seen reconstruction poll after their accounted work. Interrupted
  queries discard their local state. Requested/unverified operands retain full hashing; warm bitmap/index
  reconstruction remains a separate measured cost. Helper-terminal cancellation is calibrated after actual seen
  reconstruction, independently of the enclosing loop's final work charge and completion poll.
- **Evidence:** [set prefix qualification](set-prefix-qualification.json) binds meaningful acquisition/control
  failures, actual final sources/binaries, complete results, causal owners, integration and matched measurements.
  [Canonical skipped hashes](set-prefix-hash-qualification.json) separately binds the subsequent hash reuse,
  complete work-only regression, terminal seen control and unchanged measured reads/workspace.

### Direct singleton set bodies

- **Choice:** ordinary instant and equal-bound range clocks may pass the existing set certificate. Unequal-bound
  ranges with one effective tick already passed the old range restriction. Instant execution retains effective
  interval 1; public step 0 never replaces the replay key's evaluator interval.
- **Actual consumer:** classic histogram Count requests a singleton Vector child without bypassing ordering.
  When its descriptor output has moved to a parent, no ordered roster remains and Binary executes directly.
  Certifying an owner alone cannot reduce this work; the shared runtime predicate must admit this direct body.
- **Ordinal authority:** plain publication and projection moves preserve emitted set ordinals. If canonical
  remapping exists, entry creates OrderedRows before Binary initialization. The new condition requires both
  set certification and absent ordered metadata; it cannot skip that mapping or widen Pair replay.
- **Phases and memory:** retain complete cold discovery, preparation clearing, fresh requested and unverified
  values, original completion/truncation, duplicate controls and one accounted current-key snapshot. No values,
  history, cache field or persistent Step field is added. New instant metadata costs are measured explicitly.

## Copied grouping-label construction

- **Choice:** four copied grouping consumers construct only the selected label fields through the existing
  checked filtered-copy owner. They share the exact selection predicate with in-place grouping.
- **Scope:** copied roster groups, quantile groups, temporary top-k group keys and per-evaluation aggregate lookup.
  The roster branch that transfers owned descriptors still filters those labels in place.
- **Identity:** keep raw empty values and source order. `by` may explicitly select `__name__`; `without` excludes
  it. Ordinary `__type__` and `__unit__` fields follow grouping names. Binary result projection has different
  reserved-name/reset rules and does not define grouping selection.
- **Phases:** original grouping hashes, first hash matches, binding positions, full ranking descriptors,
  diagnostic views, output reset, parameters, warnings and numerical operations remain in place.
- **Controls:** omitted label visits and retained byte copies remain checked and accounted. A final poll follows
  the last copy charge before the grouped key returns; interruption drops every partial copied buffer.
- **Limits:** copied keys avoid discarded payloads and input-sized unused outer capacity. Input metadata,
  output cardinality, full ranking labels and transferred descriptor capacity remain. This adds no raw-history
  cache, metadata provider, persistent view or query limit.
- **Evidence:** [copied grouping qualification](grouped-label-qualification.json) binds the actual capacity red,
  final sources/executables, complete query measurements, interruption controls and containing-Agent checks.

## Completed vector-sort publication

- **Choice:** after the original numeric or label sorting callback completes, duplicate validation retains the
  direct pairwise path through 32 eligible inputs. Larger permutations use the existing accounted position
  index, storing physical input positions and comparing the descriptors' cached full hashes.
- **Identity:** duplicate equality remains hash-only, including unequal labels with the same hash. Numeric
  sorting admits float inputs; label sorting also admits native histograms. Lookup never changes the permutation.
- **Phases:** child scans, string validation and comparator acquisitions finish before duplicate validation.
  Delayed name removal bypasses this validation only when start and end are equal. Range warnings retain their
  original phase. A decisive duplicate still precedes cancellation caused by that comparison's work charge.
- **Ownership:** the temporary lookup retires before the terminal control check and snapshot publication. The
  publication checkpoint retains the existing permutation and exact time/start/end/step/position key. Same-key
  hits and invalidation keep their behavior. That change adds no raw values, history or Plan field; the subsequent
  completed-emission certificate below adds scalar delivery state.
- **Tradeoff:** the lookup reduces repeated hash comparisons but adds temporary allocations. Whole-query peaks,
  allocation traffic and elapsed time require separate measurements. Hash probing has no universal linear bound,
  and the cutoff is a measured private policy rather than a proven optimum.
- **Evidence:** the conformance guide's completed vector-sort section binds the actual owner regression, source
  phases, collision/gate/cache/error/interruption controls, complete outcomes and containing-Agent qualification.

## Completed vector-sort emission

- **Choice:** one snapshot-owned progress word distinguishes no credit, a successfully delivered contiguous
  permutation prefix, and successful full `Done`. Root consumers require full Done after the terminal control check.
  Nested singleton consumers may reuse a prefix after the Machine completes its filter check, before a parent can
  truncate that body. Sorting order or an uncompleted handoff grants no credit.
- **Eligibility:** only positive forward ranges with unequal bounds and an unchanged direct unique-float selector.
  Raw uniqueness, single-source/head/kind metadata, canonical correspondence and effective source/scope/producer
  grids must agree with the prepared evaluator header. Both bare and nested eligible sorting calls use the proof;
  computed sort inputs, shifted/modifier, instant, native/mixed and uncertified shapes keep ordinary evaluation.
- **Preparation:** install the certificate after each sort node completes its own output, projection and statistics,
  before parent consumption. After all preparation Machines drop, clear progress through the existing roster loop.
  Retain the permutation/key/certificate, including unequal bounds with one effective tick; final delivery starts cold.
- **Reuse:** root bodies keep the first complete operand traversal, including the suffix after provisional output.
  Nested bodies skip only previously delivered nonselected permutation positions; selected values are freshly acquired,
  and unvisited suffixes keep normal reads and failures when reached. Physical input positions, catalog ordinals and
  canonical output ordinals remain distinct. Sparse/absent targets create no extra selected read.
- **Ownership:** one Plan eligibility bool and one snapshot `AtomicUsize` replace the former snapshot bool without
  changing measured layouts. Positive values are contiguous prefix lengths; `usize::MAX` alone means successful Done.
  There is no new Step field. Snapshot/Strong thread traits remain intact; the Plan is serial. An old live owner
  credits its exact snapshot and cannot grant progress to a replacement or reduce later progress.
- **Memory:** no raw values, history, membership array or cache is added. The borrowed schedule proof still creates
  no temporary evaluator. The nested-prefix panel preserves all measured whole-query workspace peaks and reduces
  allocation traffic on eligible cases. These counters do not establish process RSS or a universal work bound.
- **Evidence:** [root emission qualification](sort-emission-qualification.json) retains its original complete-query,
  terminal, failure and containing-Agent identities.
  [Nested-prefix qualification](nested-sort-prefixes-qualification.json) separately binds preparation reset,
  sparse/nonmonotone owners, all prefix interruption boundaries, actual suffix and selected failures, complete
  native/ARM results, layouts, C/C++ callers and finite timings, including slower controls.


## Scheduled derived quantile rank traversals

- **Choice:** only a scheduled, dry `Kernel::Quantile` in its existing Rank phase may reuse validation as the
  consuming traversal. Capture that condition before visiting samples. Reuse the literal two-lane radix selector;
  preserve its ordered keys, signed-zero/NaN origin handling, interpolation and fixed integer state.
- **Phases:** classify provisionally during the dry visitor, await the complete child driver and `Reader::Done`,
  then update the existing dry/complete flags. The next ordinary advancement keeps its control checks, Reader
  reset/drop and `finish_pass`. The final visitor alone grants no completion. A failed driver discards query-local
  provisional counters; no rank, backing or output is published from that pass.
- **Accounting and control:** the Rank-only entry adds no poll, allocation or returned error. It charges actual
  float/native/stale samples by 4/1/0 work units immediately and reuses the existing selector charges. Keep ordinary
  receive and MAD unchanged. Partial failures therefore retain actual work without deferred `4*N` arithmetic.
- **Diagnostics:** preparation owns completed child/root annotations. Internal scheduled replays suppress their
  own builders; removing a second internal traversal removes no observable message. Preserve the remaining child
  diagnostic drain for errors and cancellation. Mixed native notes and classic nonmonotone fallback controls
  compare full kinds, spans and occurrences through accepted results.
- **Ownership:** add no Step field, sample retention, page cache, metadata owner or history array. Completed
  membership bounds counters under the existing repeatable-input contract. Fetch/materialization, provider backing,
  nonscheduled evaluation and logical sample participation keep their owners and behavior.
- **Evidence:** [rank traversal qualification](quantile-rank-traversal-qualification.json) retains the meaningful
  growth red, complete driver/control failures, final-source and byte-equivalence identities, both native profiles,
  compiled ARM owners/queries, real C/C++ callers, all required reducer/temporal outcomes and warm paired timings.
  Finite source-bound evidence does not prove every input or globally optimal performance.


## Exact rank work accounting

- **Choice:** the shared temporal quantile/MAD selector uses one existing `work(3)` call in place of three
  non-failing `work(1)` calls. The call remains after shift/mask construction and before ordered-key classification.
- **Preserved phases:** no control, callback, allocation or returned error separates the original charges.
  Retain the caller's fourth work unit, finish-pass variable charges and every existing interruption boundary.
  Do not batch charges across fallible or control-sensitive operations.
- **Arithmetic:** literal key/digit/lane operations, numerical values, MAD deviations and operation order stay fixed.
  Work uses the existing saturating counter and sticky-overflow owner. No deferred `3*N` computation is introduced.
- **Observability:** completed totals and interruption at the next poll are unchanged. A concurrent snapshot can
  skip former intermediate within-helper work+1/work+2 totals, as with other existing `work(amount)` calls.
- **Ownership:** no field, array, allocation, cache, raw history, metadata owner, cap or ABI/storage/default change.
  Source-derived savings are two mutex acquisitions per classified float; whole-query benefit requires measurement.
- **Evidence:** [work-accounting qualification](rank-work-accounting-qualification.json) binds original-charge
  invariant tests, exact complete results and nonelapsed observations, native/compiled-ARM/C owners, retained
  reducer/temporal panels and every warm timing group, including slower controls.


## Received-page accounting

- **Choice:** credit one received page and its sample count under one existing query observations guard.
  Preserve pages-then-samples order, independent saturation, sticky overflow and the existing usize-to-u64 cast.
  Exact maximum is not overflow; a zero-sample call still increments pages. No deferred aggregate is used.
- **Phases:** generic and C adapters keep their existing outer validation and Adapted exclusion. Accepted pages
  remain counted before later nested conversion, validation, consumer failure or cancellation. Invalid outer
  pages and early metadata/zero-maximum errors keep their existing no-credit behavior.
- **Observability:** a concurrent snapshot can skip the former pages-only intermediate state inside one call.
  Completed totals, next callback/control observations and poisoned-lock recovery remain unchanged.
- **Ownership:** add no field, allocation, cache, raw history, metadata owner, cap, ABI, storage or default change.
  Source-derived savings are one mutex acquisition per credited page; work units are not lock counts.
- **Evidence:** [page accounting qualification](page-credit-qualification.json) preserves old-runtime invariants,
  exact complete queries/nonelapsed observations, current native/compiled-ARM/C owners, the original failed
  baseline reader and all warmed timing groups, including separate affinity/identical-binary controls.
  Engine-accounted bytes and shared-host timings have finite scope.


## Exact C label-descriptor capacity

- **Choice:** retain per-series copied label descriptors and borrowed provider bytes. Supply the already-known final
  count as an advisory hint to the shared private vector grower. Clamp only excess geometric growth; required size
  still wins and existing capacity never shrinks. A const policy selects the original formula for ordinary growth;
  its original push body and the untracked allocator path remain. Hint arithmetic belongs only to the hinted policy.
- **Phases:** keep per-label poll, work, name/value validation and push, plus the terminal poll and existing cleanup.
  Full early reservation would allocate unused suffix capacity before later field errors; the online hint avoids it.
- **Evidence:** [capacity qualification](label-descriptor-capacity-qualification.json) records five improved native
  retained-capacity groups, including nine labels 512→288 bytes, 17 labels 1,024→544 bytes and a separate nine-label
  terminal-cancellation peak 768→544 bytes. All 17 error/cancellation phases retain work and cleanup. The 64 selected
  Debug units, twelve optimized native/ARM C/C++ callers and ordinary-query controls have explicit finite scope.
- **Boundaries:** this adds no stored field, public method, metadata provider, raw-history array or configured cap.
  Stronger descriptor-array immutability would require a separate provider contract; this change retains the existing
  copied pointers and lifetime rules. Measurements cover requested engine bytes, not allocator overhead or RSS.

## Bounded owned-label copies

- **Choice:** required full copies capture the exact iterator length; filtered copies use the input count as an
  upper bound. Feed the existing incremental hinted push after the original work check and name/value copy.
  Preserve the terminal check, one predicate visit per reached label and independent result ownership.
- **Coupled insertion:** when a copied array fills, the two local mutable append sites choose the next ordinary
  power-of-two boundary, minimum four. Compact 3/9/17-slot copies grow to 4/16/32. Shared vector growth, ordinary
  initial growth, untracked fallback, byte-copy/swap/control/error phases and existing larger capacities remain.
- **Reason:** exact byte buffers still left spare outer descriptors. Copy-only clipping exposed later 9→18 and
  10→20 mutable growth; the local insertion rule removes that measured retention regression without a global policy.
- **Evidence:** [qualification](owned-label-capacity-qualification.json) and
  [complete measured scope](README.md#bounded-owned-label-copies): 1,032 native pairs preserve complete outcomes and
  nonmemory/nonelapsed observations. Workspace peaks fall in 852, retained results in 750, with no measured increase.
  The 64-series/17-label case reduces Workspace peak 237,480→145,320 B and retained Result 224,064→131,904 B.
- **Tradeoffs:** all 20 timing groups remain, including 13 slower medians; native fixture text grows 32,128/480 B in
  release/release-min. No universal lower-peak, faster-runtime, Agent-size or RSS guarantee follows. Existing contexts
  ownership, raw paging, public interfaces, dependency set and disabled caps remain unchanged.


## Scheduled selector label ownership

- **Choice:** keep ordinary borrowed selector copies. Move the already-owned scheduled point labels and existing
  sample lease through one shared private completion helper. Remove only scheduled label views and the second
  deep copy; the initial source copy remains.
- **Phases:** retain selector identity/take-once guards and logical sample accounting, then explicitly check control
  for present and absent points. Ordinary `None` remains lazy; ordinary `Some` keeps checked copying. Clock/name
  completion, scalar parameter ordering, scheduling eligibility and the second diagnostic drain remain unchanged.
- **Reason:** a consumed private scheduled point has no external label borrower. Copying it again provides no
  additional lifetime guarantee. Moving the existing owner avoids allocation and byte-walk work without retaining
  raw samples, adding state or changing a callback lifetime.
- **Evidence:** [qualification](scheduled-label-transfer-qualification.json) and
  [measured scope](README.md#scheduled-selector-label-ownership) record 1,656 native and 1,104 compiled ARM complete
  comparisons, 654 native units/profile and actual C/C++ consumers. Workspace peaks fall in 864 pairs; allocation
  traffic/work fall in 1,218, with no measured increases. Other memory kinds and exported native counters match.
  Basic sample statistics are compared separately in 384 scheduled native C pairs; the JSON fixture omits them.
- **Tradeoffs:** all 20 timing groups remain, including five slower medians. The 64-series/17-label map improves
  1.28–1.34×; the 32 KiB-label release-min case slows 3.55%. Native fixture text grows 528/944 bytes. No universal
  lower-peak, faster-runtime, Agent-size or RSS guarantee follows. Public interfaces, contexts authority, raw paging,
  dependency set and disabled caps remain unchanged.

## Scheduled evaluator frame reuse

- **Choice:** move the metadata-only scheduled reader's already-held frame allocation through a shared fresh
  constructor. Every point reruns `Frame::new` and all Machine defaults. Return only drained capacity no larger than
  the donor; discard grown capacity without eager replacement. The separate raw reader retains its required frame.
- **Phases:** take frames and annotations into the callback before fallible work. Keep parameter/index/filter/
  selector/control/statistics phases, the visitor and second diagnostic drain. Release unused selected points before
  returning empty storage; errors and panics drop active state before provider unwind. No raw values survive reuse.
- **Evidence:** [qualification](scheduled-frame-reuse-qualification.json) and
  [measured scope](README.md#scheduled-evaluator-frame-reuse) retain the old-runtime allocation failure, fresh-state/
  growth/page/failure controls, 1,836 native and 1,224 compiled ARM original comparisons, 657 native units/profile
  and actual C/C++ consumers. Workspace peaks fall in 1,296 native pairs and allocation traffic in 1,374; none rises.
- **Tradeoffs:** the 256-tick map halves allocation traffic but lowers workspace peak by only 960 bytes. Shared-host
  timings establish no throughput gain; all 20 groups, including nine slower medians, remain. Native fixture text
  grows 41,008/2,976 bytes in release/release-min. No Agent-size or RSS result follows. Ordinary constructors, contexts
  ownership, paging, interfaces, dependency set and disabled caps remain unchanged.

## Completed mapped rate join discovery

- **Choice:** extend the existing binary operand proof through pure Float functions, vector/scalar operations,
  unary expressions and parentheses ending at the ordinary direct range-rate proof. Both terminals use the existing
  `RangeFunction::Rate` class: `rate`, `increase`, `delta`, `irate` and `idelta`. One private predicate retains the
  established `rate` short circuit before typed classification; its registered-name membership is tested. Return immediately for
  already-supported raw, earned-marker and direct rate-family operands. Share the existing wrapper-purity predicate;
  preserve its scheduled caller's child, earned-marker, then scalar-purity order.
- **Authority:** use the original output header for producer/source/scope grid checks at every effective wrapper.
  Keep fixed/invariant/matrix exclusions and the direct rate-family arity, selector, extended-window and checked endpoint
  rules. Resolve left before right and propagate interruption immediately. Window outputs receive no raw-point marker.
- **Evaluation:** the existing Pair requests the original operand ordinal and evaluates its complete subtree,
  including fresh scalar parameters. Full successful cold discovery, Float/source/head/alias uniqueness, exact
  scope keys, matching cardinality, diagnostics and cancellation retain their existing owners. Grouped joins can
  use the same existing proof; unsupported transformations keep ordinary evaluation.
- **Memory:** no raw value, history, new cache or Pair representation is added. Newly eligible queries can use the
  existing accounted Pair maps and one-step membership bitmap; their small-query peak cost must remain visible.
  Lower cumulative allocation traffic does not imply lower peak memory or process RSS.
- **Original mapped-rate evidence:** [qualification](mapped-rate-join-qualification.json) and
  [measured scope](README.md#completed-mapped-rate-join-discovery) bind complete pinned expressions, the causal
  old-runtime work regression, calibrated source/output failures, actual C/C++ consumers and optimized timings.
  Public interfaces, contexts authority, storage, raw paging, dependencies and disabled caps remain unchanged.
- **Family evidence:** [range-rate qualification](range-rate-join-qualification.json) and the
  [measured scope](README.md#completed-range-rate-join-discovery) separately bind all five numerical dispatches,
  mixed-family joins, resets and typed-tail behavior. Numerical and diagnostic owners remain unchanged; equal-function
  ratios alone do not establish correct dispatch. Earlier rate-only measurements retain their original identities.
- **Measured result:** 804 native comparisons, 18 supplemental candidate boundary outcomes, 536 compiled ARM
  comparisons, 659 native units/profile and actual C/C++ consumers pass. Four 64-pair mapped shapes improve
  9.3–13.2× in the fixed shared-host timing panel; no live DBengine or concurrent-throughput claim follows.
- **Tradeoffs:** 96 of 804 workspace peaks rise by at most 312 B; 42 allocation-traffic observations rise by at
  most 64 B and 60 work counts by at most 18 units. The representative clamp workload lowers work, reads and
  allocation traffic while keeping its peak unchanged. Fixture `.text` grows 1,600/720 bytes in release/release-min.


## Completed per-series fold join discovery

- **Choice:** both direct and mapped ordinary-window terminals share `is_replay_window_function`. Preserve the
  established `rate` fast branch and classify other names through the existing `RangeFunction` owner. Admit
  per-series Fold functions alongside Rate; exclude `absent_over_time`, whose actual evaluator produces a global
  synthetic row. Numeric reducers keep their existing scalar/temporal owners.
- **Boundaries:** retain every original arity, selector, effective clock, invariant, fixed-time, extended-window
  and checked endpoint guard. Full successful cold discovery and the Float, source, head, hash, exact-key and
  cardinality checks remain authoritative. Failed certification uses the ordinary path.
- **Values and names:** Pair evaluates the original subtree and physical series ordinal, including fresh selected
  reads. First/Last preserve names; optional timestamp functions require their explicit parser options. No raw-point
  marker, retained history, new cache, Pair layout, provider, metadata, storage or ABI change is introduced.
- **Evidence:** [qualification](fold-join-qualification.json) and
  [measured scope](README.md#completed-per-series-fold-join-discovery) bind distinguishing complete original outcomes,
  optional positive paths, global absence, mixed/native states, failure phases and actual C statistics. Single-ID
  boundary controls establish clock/fallback outcomes; they cannot install a multi-row Pair.
- **Costs:** reuse existing membership metadata. Small-query peaks may rise; allocation traffic, tracked peaks,
  fetched samples and timings are reported separately. Neither reduced work nor allocation traffic proves lower RSS.

## Grouped aggregate contributors

- **Choice:** ordinary aggregate operands retain accounted group-to-child output ordinal membership, built during
  existing label projection. The existing hash-only grouping authority, alias unions, first labels and source order
  remain unchanged. Membership retains identities; values still pass through the original reducers.
- **Admission:** the initial cohort builds the private plan along eligible root arithmetic/comparison operand paths,
  including admitted Float maps, ordinary nested aggregates and supported raw/window leaves. Keep the actual Float output,
  cardinality and complete Pair proof. The aggregate certificate never grants a one-raw-point scheduling marker.
- **Histogram consumers:** each visited `histogram_quantile`, `histogram_fraction` or optional
  `histogram_quantiles` seeds demand from its resolved vector argument. Demand travels on the existing accounted
  preparation-task stack through ordinary aggregates and supported maps; scalar/string parameters, unsupported
  edges, invariants and
  clock boundaries stop propagation. An inner histogram seeds its own local demand independently of outer wrappers.
  The existing selector/grid proof still controls each plan. Pair-only Float/cardinality restrictions remain with
  joins; native/mixed histogram consumers do not inherit them. No persistent per-node context array is added.
- **Completion:** a full child traversal must reach successful Done for the exact clock before membership filters
  later requests. Clear preparation credit before final delivery. Singleton group lookup is direct; nested requests
  use sorted/deduplicated accounted ordinal unions without changing arithmetic traversal order. RefCell borrows stay
  inside each operation and nested plans own different active unions.
- **Ownership and failures:** selected values follow ordinary acquisition, including existing completed-window-fold
  reuse. Preserve cold nonmember failures, genuinely read warm selected failures and provisional output cleanup.
  Exact-key changes force full traversal. No raw-history array, copied child labels or aggregate payload cache is added.
- **Costs and evidence:** the optional roster pointer has a per-node cost even for standalone queries. Demanded plans
  add member/group metadata and temporary construction storage; lower allocation traffic does not imply a lower
  peak. The [procedure and measurements](README.md#grouped-aggregate-contributors) and
  [qualification](group-contributor-qualification.json) keep complete original outcomes, actual callback phases,
  resource counters and fixed paired timings separate from historical broader cohorts.

## Wrapped grouped joins

- **Choice:** resolve the first eligible arithmetic/comparison vector join through existing pure scalar/Float,
  unary, parenthesis and preprocessing paths. Demand the established contributor plans from that join. Keep the
  original clock, kind, uniqueness, cardinality and supported-leaf checks; unsupported shapes use ordinary evaluation.
- **Ownership handoff:** check original input aliases before label projection moves or copies their descriptors.
  An optional candidate marker carries only that eligibility. Final output certification consumes it; unsuccessful
  admission drops it. The marker cannot authorize input or value reuse.
- **Discovery:** publish accounted active input membership only after both original operand streams reach Done for
  the exact ReplayKey. Preserve suppressed and unmatched inputs and their strictly increasing physical ordinals.
  Restore sparse inputs in that order. Preparation, replacement or key changes clear completed progress.
- **Pair credit:** retain static pair identities, their original result hashes, and two bits per pair for unknown,
  suppressed or kept. Grant credit only after the original operand, label, reservation, seen and control phases
  succeed. A known non-target pair may reconstruct its original reservation and kept-only seen effect without
  reading its values again. The requested pair always follows fresh ordinary evaluation.
- **Completion:** a processed prefix does not prove an unvisited suffix or full Done. If a fresh target becomes
  suppressed, continue ordinary traversal and preserve later operand failures. Do not replace a prefix with direct
  singleton output; surrounding operators may reject a sample or drain additional input.
- **Memory and limits:** reuse the existing discovery bitmap and replay wrapper. Optional boxed metadata keeps
  the maximum replay-enum and evaluator-Step layout unchanged. No raw samples, retained values, child labels,
  payload cache, dependency, ABI, provider, contexts or storage change is added. Metadata restoration and prefix
  walks can still grow quadratically with series count; no linear-runtime or optimality claim follows.
- **Evidence:** the [procedure](README.md#wrapped-grouped-joins) separates complete original outcomes, declared
  ordering-policy qualification, callback/control phases, query-owned memory and paired timing measurements.

## Completed pair-metadata traversal

- **Choice:** extend the existing optional pair-discovery plan with direct traversal of completed operand
  membership. Eligible warm requests avoid rebuilding active-row, signature, reservation and seen tables and
  rescanning an already verified physical prefix. Cold discovery, original root replay and set operations retain
  their existing owners. No additional expression shape is admitted by this change.
- **Certificate:** require the existing Float, alias, signature and cardinality proof, then prove that every
  statically matching many-side row has a frozen output binding and every bound output has a distinct canonical
  full hash. Include suppressed outputs. A globally unmatched row is allowed; an unbound possible match or a hash
  collision retains ordinary evaluation and its original reached errors. Temporary proof indexes are accounted
  and released before execution.
- **Progress:** the exact-key completed membership bitmap and successful two-bit pair credit remain authoritative.
  Advance a physical many-side cursor only across inactive, unmatched or successfully credited rows. Choose the
  earlier of the next unverified pair and a fresh requested pair. Cache ordinals do not order traversal. Backward
  requests remain fresh; an absent or freshly suppressed target continues into unknown suffixes and ordinary
  Finished. A full, unscoped traversal reads every active pair freshly.
- **Phases:** the stronger certificate excludes reservation and seen conflicts, so only this warm mode omits those
  tables. Original child reads, labels, arithmetic, output and terminal control remain. Publish the cursor after
  control checks; publish credit only after successful original phases. Drop all temporary borrows before child
  actions. Preparation and every replay-key change clear cursor and credit together.
- **Ownership:** a private enum stores either ordinary saved hashes or the certified cursor. Successful
  certification releases the obsolete hash buffer. An ordinary prefix consumer of a certified plan derives its
  original hash from immutable frozen labels. No raw values, child-label copy, timeline or second page cache is
  retained. AMD64/ARM64 Step grows from680 to688bytes, and ARM32 from496 to504bytes. The other measured
  replay/plan layouts remain unchanged on each target.
- **Complexity and scope:** certified physical-prefix selection is amortized over rows and requests for an exact
  key. This does not establish linear whole-query execution: child evaluation, fallback paths and other metadata
  work remain. Public API/ABI, storage, contexts, math, dependencies, temporary backing and disabled implicit caps
  are unchanged. See the [qualification procedure](README.md#completed-pair-metadata-traversal) for complete
  results, costs and the separate original/order-policy reports.
