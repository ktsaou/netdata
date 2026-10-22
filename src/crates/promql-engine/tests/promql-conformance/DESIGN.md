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
  Netdata supplies the concrete backend through its containing provider.
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

An ordinary selector inside a generic body can evaluate only its requested logical instant when the checked
nonwrapping schedule, exact grid membership and positive lookback prove that earlier Plain projections carry no
state. The local window and driver use the same admission. Full request headers, prepared offsets, raw source clocks,
sample indexes and the ordinary predecessor search remain intact. Fixed bindings, histogram-statistics decoding,
smoothed selectors and unsupported clocks keep their existing traversal. This adds no retained samples or cache.
Sparse data can require an additional physical Seek; its actual error remains observable. See
[single-clock Plain selectors](README.md#single-clock-plain-selectors).

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
  binary signatures and result-cache keys check cached hashes followed by complete canonical field identity. Arrays keep their
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

#### Four-byte position slots

Ordinary tables store one `u32` per logical slot. A position whose encoded value needs more bits promotes the
table to two words per slot; the complete `usize` position remains authoritative. Hashes and caller predicates
retain their full identity rules. Empty and singleton indexes keep their existing allocation-free representation.

One tracked buffer and the existing state word preserve the header budget. Interpret the width marker only for
allocated tables: a singleton can use every state bit. Table counts have spare bits under the buffer's existing
checked allocation bounds. ARM32 needs one word for every representable position; no position cap is introduced.

Select width before probing or visiting. A width-only promotion copies physical bucket/signature offsets and
does not invoke rehash callbacks. A simultaneous capacity increase preserves the original rehash traversal.
Dense conversion retains its required-sparse-allocation comparison in the same width. Replacement allocations
remain private until success, with both capacity leases live during copying and unchanged work per logical slot.

Cold rebuild and dense-conversion loops borrow their hash/key callbacks through shared routines. Hot equality
predicates remain specialized. This shares growth code without boxing callbacks or adding persistent metadata.

The ordinary table payload halves on 64-bit targets. Whole-query peaks can stay unchanged when other metadata
dominates. A rare width-only promotion temporarily holds compact old plus wide new storage and can exceed an
always-wide steady table; this overlap remains accounted. Linked code, callback dispatch and stack costs are
separate from heap payload. See [the finite slot qualification](README.md#four-byte-position-slots) for measured
query, memory and code tradeoffs; raw histories and public interfaces are unchanged.

#### Mapped label and kind identity

`UniqueKinds` retains accepted `(full label hash, kind)` tuples in encounter order. Both mapped evaluation and final
delayed-name duplicate cleanup use this owner at their original phases. Kind zero is an exact identity value, not a
wildcard. Unequal label bytes with the same full hash retain the pinned hash-only duplicate behavior.

The original vector handles empty and singleton cases. Only a second distinct accepted tuple creates an accounted
boxed `PositionIndex`; the index stores positions and uses complete tuple equality on every target. A duplicate of
the first tuple allocates no index. Checked tuple growth and index rebuilding retain overlapping allocation charges.
Terminal cancellation polling precedes duplicate errors and follows successful insertion. Operation, diagnostic,
omission, name-removal and per-tick ownership rules remain with their existing callers.

The native frame remains 240 B. Plural queries pay additional index/header metadata; no raw samples or output values
are cached here. Removing this scan does not establish linear total execution across other metadata/replay owners.
See the [mapped identity procedure](README.md#mapped-label-identity-lookup) for causal controls and
complete-query costs.

#### Final output identities

Generic final merging uses a query-wide `OutputIdentities`: append-only full hashes plus an inline
`Option<PositionIndex>`, initially `None`.
Append positions already are first-emitted IDs, so the separate ID field is removed. Equality uses the complete
64-bit hash and ignores value kind. Full-hash aliases preserve the first callback's labels; missing ticks retain IDs.
Canonical roster ordinals do not generally encode first productive output.

Up to two identities use a bounded scan of their full hashes. The third distinct identity builds a local index
containing both prior IDs and the candidate. Publish it only after construction succeeds, then immediately append
the reserved candidate hash. Interrupted construction leaves the original two hashes searchable and releases the
temporary index. Larger sets use the existing checked index insertion and rebuilding. There is no unused startup
index or separate box.

The owner is created only after scheduled output declines, for the existing delayed-name/root-unary-minus gate.
The same immutable gate remains the per-output dispatch condition, before borrowing the mutable owner.
Evaluation, name cleanup, duplicate validation and checked label views precede lookup. Checked key reservation
precedes index mutation; successful insertion immediately appends the reserved key. The original decisive hit has
no terminal lookup poll; checked output observes cancellation. Failure discards provisional output and all metadata.

Keys occupy 8 B versus the old 16 B tuple on measured native/ARM targets. Up to two identities allocate no slots;
larger tables add metadata and overlapping rebuild charges. The inline owner is once per generic query, not per
evaluator frame; it introduces no raw/value/label cache. Actual accounting, type layouts and linked costs are in the
[output identity procedure](README.md#final-output-identity-lookup).

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

#### Dense owned-result identity lookup

The convenience Collector keeps a private positional-ID proof: every stored row has `series[index].series_id ==
index`. While that proof holds, a lower ID is a direct hit and an ID at or beyond the length is absent. Both cases
charge one checked lookup unit. Successful append preserves the proof only when the new ID equals the old length;
publish that update after owned label copy and row insertion. Sparse, permuted and arbitrary caller IDs retain the
existing identity scan. There is no additional index, per-row metadata, value cache or raw-history owner.

A successful begin resets the proof. An interrupted checked begin preserves the previous output. Invalidate before
fallible Matrix sorting, since cancellation may follow actual swaps. First labels, point arrival order, distinct IDs
with equal labels, pinned sort provenance, partial-copy ownership, abort, take and independent result release keep
their existing behavior. The proof never escapes with the collected result. Owned output still grows with the
answer; this specialization does not add a total query-memory bound.

The [dense collection procedure](README.md#dense-owned-result-collection) separates actual Collector callback work
from complete-query timing. Rust collection and the C convenience collector use this path. Netdata's HTTP streaming
consumer has its own indexed row lookup; this change does not accelerate that lookup.

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
  ambiguous, nested delayed-name, invariant and unsupported clock shapes retain ordinary replay. Initial validation and
  error priority remain complete; errors tied to omitted physical operations follow the existing provider policy.
- **Grouped ranges:** existing direct/rate/earned Float operand proofs can admit `group_left`/`group_right`.
  Keep unique full float rows on both sides and unique matching signatures on the one side. Enumerate every
  many-side row; reject ambiguous histories, missing output bindings and conflicting cache ordinals. Pair positions
  remain in original left/right order while frozen keys use many/one label order. Existing cold validation,
  successful Done, exact-clock membership and fresh selected reads remain the same owners; no state field is added.
- **Deferred-name final roots:** use the structural predicate only for final-root non-prefix certification.
  Keep the delayed-name wrapper for target discovery, grouped demand, nested PairCandidate and prefix paths.
  Reuse the same complete unique-Float, cache-binding and clock proof. Actual root Matrix children drain through
  Done before sharing membership; fresh selected values, next-clock discovery and final name/duplicate/identity
  owners remain. The option stays disabled by default. No persistent layout, raw value/history or reader is added.
- **Root proof and costs:** [finite qualification](root-delayed-name-qualification.json) retains 80 result-first causal
  controls, actual root cold/suffix/selected/new-clock errors, name aliases, 13,152 exact outcomes, portable/C ownership,
  all memory/statistic/read costs and serial timing samples. Optimized artifacts retain their actual pre-test-cleanup
  source; final checks explicitly prove production-source equivalence after one identity cast is removed in cfg(test).
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

Current instant-rate acquisition is documented under
[completed instant rate float windows](#completed-instant-rate-float-windows).

This section records the earlier acquisition-only qualification. Current zero-or-one-reset behavior
is documented under [single-reset correction ownership](#single-reset-correction-ownership).

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

## Direct classic quantile probes

- **Choice:** after the ordinary full quantile correction sweep, reuse existing prepared probes when raw/coalesced
  positions coincide and neither copied result repair flag is set. Repaired, coalesced and fraction cases retain
  their generic sweeps. Invalid-quantile, missing-infinity, short-group and zero-total exits stay with their owners.
- **Proof:** `groups == bounds.len()` makes sorted raw positions equal coalesced positions. Without precision or
  monotonicity repair, correction returns each original current count unchanged, including its bits. NaNs,
  infinities and signed zeros need no added restriction. Binary-search predicates and interpolation stay pinned.
- **Transition:** complete the original `finish_run/search/reset_sweep` first, then admit only the successful
  nonterminal total transition using `self.result`, not reset correction scratch. Redispatch at the generic loop
  top before indexing another Count. Remaining probes and endpoints still request fresh child values.
- **Ownership:** reset releases obsolete correction/count/previous sample owners and preserves total/result/rank
  state. Reuse is confined to this Replay at this instant. No field, buffer, index, cache or raw-value array is added.
  Logical source participation remains unchanged; actual numerical reads and resource traffic describe the new work.
- **Controls:** the complete first sweep retains input errors. Prepared traversal polls before its next action;
  cancellation after promotion releases all owners. Errors from remaining physical operations retain their existing
  precedence; the provider contract does not promise errors at omitted callbacks.
- **Evidence:** the [owning guide](README.md#direct-classic-quantile-probes) describes complete helper/original,
  duplicate/repair/control, plural dispatcher, caller and finite whole-query qualification. Fewer Count requests do
  not by themselves establish lower peak RAM or general throughput.

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

- **Choice:** retained key-owning grouping consumers construct only the selected label fields through the existing
  checked filtered-copy owner. They share the exact selection predicate with in-place grouping.
- **Scope:** copied roster groups, quantile groups and temporary top-k group keys. Per-evaluation aggregate lookup
  now [borrows labels](#borrowed-aggregate-lookup-labels); the earlier four-path cohort keeps its source attribution.
  The roster branch that transfers owned descriptors still filters those labels in place.
- **Identity:** keep raw empty values and source order. `by` may explicitly select `__name__`; `without` excludes
  it. Ordinary `__type__` and `__unit__` fields follow grouping names. Binary result projection has different
  reserved-name/reset rules and does not define grouping selection.
- **Phases:** original grouping hashes, first hash matches, binding positions, full ranking descriptors,
  output reset, parameters, warnings and numerical operations remain in place.
- **Controls:** omitted label visits and retained byte copies remain checked and accounted. A final poll follows
  the last copy charge before the grouped key returns; interruption drops every partial copied buffer.
- **Limits:** copied keys avoid discarded payloads and input-sized unused outer capacity. Input metadata,
  output cardinality, full ranking labels and transferred descriptor capacity remain. This adds no raw-history
  cache, metadata provider, persistent view or query limit.
- **Evidence:** [copied grouping qualification](grouped-label-qualification.json) binds the actual capacity red,
  final sources/executables, complete query measurements, interruption controls and containing-Agent checks.

## Parameterized aggregate preparation

- **Ranking:** prepare_rows computes the existing checked borrowed grouping hash at every raw field count.
  Its index stores hashes/positions; full input owners remain for selection, sampling and result labels.
- **Quantile:** the first key must miss and be retained, so it takes the original copied constructor. Later
  <=64-field keys reuse the existing raw-position mask and copy only misses. Above 64 fields preserve original
  copy/hash/lookup/drop-or-retain. Share copied construction with that fallback; no variable mask or retained field.
- **Contracts:** preserve raw empties/order/delimiter aliases, first labels, literal0 versus empty digest,
  hash-only identity, input buffers/ordinals/flags/kinds/sources, grouping-before-parameter validation and final
  publication. Failures consume inputs and release local groups/bindings/partial keys.
- **Controls:** retain 400 original causal scenarios/four exact regression bodies and 24 raw cases. Calibrate
  first-prefix work/acquisition before actual second-key hash/terminal/masked-copy and copied-fallback controls;
  second 64-field keys exercise raw bit63 with an empty value. Require complete-query original work/eight memory
  observations for all literal singleton quantiles and larger-field fallback cases.
- **Measured scope:** constructor acquisition, complete-query acquisition/peak, work, fixed timing, shared
  compiled helper code, linked size and transient layout are distinct. Preserve the initial fully qualified
  but unaccepted candidate and all slower controls; five extra work units do not explain an 11.360-microsecond elapsed cost.
- **Evidence:** [parameterized preparation qualification](parameterized-preparation-qualification.json) binds
  current native/foreign/original/reducer/literal/default C/C++ outcomes and exact historical attribution.
  No public language/ABI/provider/contexts/storage/math/dependency/default/cap change or universal throughput claim.

## Prepared grouping key ownership

- **Choice:** retained-source preparation hashes <=64raw label fields while recording the existing named-first
  selection in one transient u64. A miss consumes those raw-position bits in checked copying; a hit keeps the
  first owned key. Above64fields retain original copy/hash/probe and hit cleanup, with no query cap or variable
  selection array. The consumed-input transfer branch stays byte-identical.
- **Identity/provenance:** retain raw empties, source order, delimiter aliases, literal-zero versus empty digest,
  hash-only first labels and encounter order. Record every canonical child ordinal, including hits; source
  binding ordinals are different. Zero/singleton/plural contributor proof semantics stay unchanged.
- **Control/ownership:** share checked predicate/digest with ordinary group_hash through a static no-op observer.
  Hash/copy/index/replay/final publication remain checked. Miss copy keeps raw-count capacity hint and final
  poll. Its borrowed source cannot outlive local preparation; no permanent field or metadata owner is added.
- **Measured trade-off:** first candidate's repeated unique-miss selection caused a retained5.615percent fixed
  follow-up slowdown; it was not accepted. Refined selection-word path reuses those decisions, while >64fallback
  keeps exact original work/memory. Record constant transient layout, shared ordinary compiled code, every
  slower row and linked size cost. Constructor acquisition, query peak and RSS are different quantities.
- **Evidence:** [prepared grouping qualification](group-preparation-qualification.json) retains the causal old
  constructor failure, complete raw/contributor/0–128-field/control checks, native/installed foreign/C/original/
  reducer-policy/literal outcomes and fixed timings. Original strict differences and failed test reference
  remain separate. No public language/ABI/storage/provider/math/dependency/default or implicit cap change.

## Borrowed aggregate lookup labels

- **Choice:** ordinary aggregate receive hashes selected immutable sample labels directly. The existing prepared
  group remains the owner of its grouping labels; lookup needs no copied key or diagnostic view array.
- **Identity:** use the same `by`/`without` predicate and source-order name/255/value/255 encoding as owned keys.
  Keep raw empty values and hash-only first matches, including delimiter aliases. No grouping and `by()` use
  literal zero; other empty selections retain the ordinary empty digest. Full-label identity encoding is separate.
- **Diagnostics:** after `Group::add` and diagnostic-node/builder gates, borrow the checked metric name directly
  as the annotation argument. Child-span resolution follows it. Sample labels remain alive and unchanged;
  annotation formatting consumes their bytes synchronously. Native operation errors retain their earlier phase.
- **Controls:** check/count examined labels, checked grouping-name comparisons and selected hash chunks. Poll
  after the final charge. Removed copies remove actual copy work; checked warning lookup can add comparison work.
- **Ownership:** prepared copied/transferred descriptors and ranking labels retain their existing owners.
  [Aggregate-result borrowing](#borrowed-aggregate-result-labels) handles finalization separately. Lookup adds no
  retained field, metadata provider, shared-label owner or raw-history array.
- **Evidence:** [borrowed lookup qualification](borrowed-aggregate-qualification.json) records exact complete
  operator/reducer results, selected C/target owners and actual whole-query allocation, timing and linked costs.

## Borrowed aggregate-result labels

- **Choice:** ordinary `Group::finish` borrows clean immutable roster labels through the existing private
  borrowed/owned representation, generalized as `SampleLabels`. Copies occur at real ownership boundaries.
- **Normalization:** numerical and native-histogram finalization precede the checked empty-value scan. An empty
  raw value enters the existing copied/reset fallback; clean labels retain the original frozen owner. Group
  hashes, source order, first matches and final output identity remain unchanged.
- **Lifetimes:** evaluated samples and retained actions use the frozen roster lifetime independently of a mutable
  `Machine` frame. Short replay borrows stay short. No unsafe lifetime extension or provider borrowing is added.
- **Mutation:** borrowed no-op reserved removal validates the initial name and scans without copying. Actual
  removal materializes checked/accounted ownership, then applies existing policy. Owned removal keeps its path.
  Failed materialization retains the original borrowed owner; partial copied buffers retire normally.
- **Retention:** existing-descriptor lookup precedes materialization. New descriptors and `count_values` own their
  labels. Conversion failure after provisional index insertion discards that complete local preparation owner.
  Selector, mutable-cache and ranking producers keep their existing owners; public collection remains
  independently owned after query/provider/internal lifetimes end.
- **Resources:** no raw-history array, global metadata cache or shared heap owner is introduced. Checked scans and
  copies account actual work; allocation traffic, peak bytes, layouts and linked code costs are separate measures.
- **Evidence:** [aggregate-result label qualification](aggregate-result-labels-qualification.json) binds meaningful
  ownership/interruption controls, complete expressions/reducers, actual C consumers, three-target layouts and
  matched query measurements. Historical attempts retain their own source identities and dispositions.

## Borrowed projection-result labels

- **Choice:** productive `State::Labels` emission borrows exact immutable prepared descriptor labels through
  `SampleLabels`. It does not apply aggregate empty-value normalization to projection bytes.
- **Phases:** the original binding lookup finishes before terminal control and borrowed publication. Name flags,
  output ordinal and source clocks retain their existing meaning.
- **Lifetime:** the frozen roster outlives Machine frames, re-entry and pending values. Actual mutation and retained
  descriptors or public results keep their existing checked/accounted materialization and failure cleanup.
- **Resources:** no new field, heap owner, metadata provider or raw-history array. Allocation traffic, query peaks,
  elapsed engine time and linked code size remain separate measurements.
- **Evidence:** [projection-result qualification](projection-result-labels-qualification.json) binds complete
  expressions, actual native C owners, focused ARM executions and all slower/memory/code-cost rows. Compressed
  query captures retain raw/compressed hashes; resource baselines match page, target and profile.

## Borrowed binary-result labels

- **Choice:** Set results borrow immutable side-descriptor labels; frozen Pair cache hits borrow the exact entry
  labels through existing SampleLabels. Mutable preparation hits/inserts retain owned labels. No new field,
  shared owner, representation or cache is added.
- **Identity:** retain exact frozen bytes, including empty values. Aggregate normalization does not apply here.
  Undelimited keys, complete-key collision checks, first-writer results, false comparisons, source ordinals,
  name flags, hash-only duplicates and cardinality remain unchanged.
- **Phases:** poll at the replaced Set copy and after successful frozen lookup. Missing-key errors retain their
  earlier phase. Hashing, reservation, values, diagnostics, seen tracking and successful replay credit stay intact.
- **Pair cache keys:** one lookup owner validates/hashes both complete borrowed operand streams before probing.
  Ordinary entries retain original-side row positions in one immutable Plan; same positions prove identity only
  after validation/hash. Different positions compare flattened fields across splits. Exact Encoded fallbacks
  compare canonical prefix/payload bytes to exhaustion. Preserve raw empty/opaque fields, many/one order,
  collisions, first false/true writers and terminal controls. Transient matching signatures use the separate borrowed owner below.
- **Filled ownership:** matched synthetic operands retain an accounted boxed encoded vector, constructed before
  projection/output copy/insertion. Ordinary entries retain no operand-byte copy; preparation hits materialize
  only independent output labels. Mutable-to-frozen handoff stays with the same Plan/node. No metadata provider,
  raw history, source-label validation assumption or public interface is added.
- **Costs and proof:** [compact operand-key qualification](operand-key-qualification.json) retains actual
  preparation retention/hit controls, complete fill aliases and partial cleanup, 13,152 original/before outcomes,
  actual native callers, portable layouts and every measured memory/latency/linked cost. Compact ordinary entries
  trade 24/16 B header savings for one extra 72/48 B allocation on filled entries: net 48/32 B before capacities.
  The [earlier frozen-key checkpoint](frozen-pair-key-qualification.json) retains its historical attribution.
- **Fresh Pair digest:** reuse the first successful full 64-bit result hash after reservation. Labels remain
  immutable until emission; false filtering still follows the original hash/reservation. Seen/prefix/pending
  checks remain. Set hashes once, without moving any observable work or fallible phase. This is a local tuple
  value, not a retained cache. See [fresh Pair result digests](README.md#fresh-pair-result-digests).
- **Lifetime:** the frozen Plan outlives evaluated samples and Machine frames/re-entry. OrderedRows keeps its
  conditional canonical replacement; real mutation, retained descriptors and public output materialize independent
  checked/accounted ownership. Borrowing never reaches the mutable preparation RefCell.
- **Evidence:** [the binary-result procedure](README.md#borrowed-binary-result-labels) and its finite qualification
  bind causal ownership failure, complete original outcomes, terminal controls, actual native C statistics and
  matched memory/timing/linked costs. Fixture statistics, foreign focused execution, query allocations and actual
  default-library/live throughput retain their distinct scopes.

## Borrowed matching-signature labels

- **Choice:** temporary signature keys borrow immutable Side label fields during Plan construction. A complete
  filtered sized-field digest validates selected incoming fields before probing. Contiguous selected spans avoid
  repeated predicates; interior omissions use independent fallible cursors. Do not normalize raw empty/opaque
  fields, reorder fields or substitute grouping's delimiter hash.
- **Reuse:** one named-first raw-name predicate serves matching encoding/views, matched-label filtering and the
  three grouping callers. Keep append exclusion/None branches, matched reset, grouping's literal-zero hash and
  caller control/work phases in place. Projection uses different semantics and retains its own predicate.
- **Ownership:** disjoint row/signature field borrows allow the sides to keep their label owners while signature
  arrays grow. Drop temporary views/index before sides move. Preserve first physical IDs, complete collision
  checks, index-before-signature publication and partial/terminal control cleanup. No view escapes construction.
- **Costs:** actual key headers become32/24 B native64/ARM32, replacing56/40 B and per-key byte buffers. All72
  constructor controls pass with width-independent acquisition. Whole-query peaks remain unchanged in the13,152
  binary pairs;11952reduce allocation traffic. Default C text grows28960/424 B release/size. These are explicit
  traffic/code trade-offs, not a universal reduction of resident memory or proof of throughput.
- **Evidence:** [the procedure](README.md#borrowed-matching-signatures) and
  [finite qualification](matching-signature-qualification.json) retain original/before/literal outcomes, native C
  statistics, portable controls, layouts and every timing/resource/linked cost. Existing caps remain disabled.

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
  float/native/stale samples by 4/1/0 work units immediately and reuses the existing selector charges. This traversal
  reuse does not apply to ordinary receive or MAD. Partial failures retain actual work without deferred `4*N` arithmetic.
- **Diagnostics:** preparation owns completed child/root annotations. Internal scheduled replays suppress their
  own builders; removing a second internal traversal removes no observable message. Preserve the remaining child
  diagnostic drain for errors and cancellation. Mixed native notes and classic nonmonotone fallback controls
  compare full kinds, spans and occurrences through accepted results.
- **Ownership:** add no Step field, sample retention, page cache, metadata owner or history array. Completed
  membership bounds counters under the existing repeatable-input contract. Provider backing and logical samples
  keep their existing owners; generic histogram probes follow the type-union exclusion documented below.
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


## Early completion of temporal ranks

- **Choice:** a whole successful replay may finish uniform populations or skip proven constant groups of key bits.
  Retain two fixed counter tables, both ordinary rank/control validations and the sixteen-pass final fallback.
- **Representation:** reuse full per-lane normalized representatives, masked admission and seen bits. One byte
  starts at 64 and records the exact minimum common leading length within the admitted lanes. A zero key is valid.
- **Continuation:** reconstruct the selected counted digit first, clearing lower bits. Jump only when the shared
  length proves more than the ordinary next nibble, bounded by nibble 15; only then fill bits from representatives.
  Without a true jump, a first representative can have an unselected digit. The final nibble returns reconstructed
  keys without asking for another pass.
- **Completion:** both rank/control checks and whole child/Reader success precede publication. A single uniform
  lane, partial driver or final visitor grants no completion. Preserve current-pass errors and cancellation.
- **Numerics:** normalized keys do not supply NaN payload/zero-sign provenance. Membership, ordinal Fetch,
  provider-owned exact sort backing, MAD deviations and interpolation retain their operation order and owners.
- **Accounting:** receive retains infallible `work(3)` and caller control boundaries. Extra leading-zero instructions
  are CPU cost, not new logical work units. Fewer physical passes may reduce reads/work without changing logical
  queryable totals. Omitted future replay callbacks are not a fixed physical-roster contract.
- **Controls:** preserve old adjacent/subnormal/clipped numerical cases as acceleration controls. Genuine `q=0` prefix
  and majority-zero MAD ladders retain all 16 rank passes and 33 MAD scans, late faults and clipped seek targets.
- **Ownership:** the two state bytes reuse measured padding. Selector, both medians, Kernel and enclosing Step/Frame
  sizes stay unchanged on AMD64/ARM32/ARM64. No new allocation, raw/history cache, backing policy, cap or public
  contract. Type sizes do not establish whole-function stack or process RSS bounds.
- **Evidence:** [common-prefix qualification](rank-prefix-qualification.json) binds complete original/native/foreign/C
  results, generated page-bounded growth, no-jump and accelerated timings, and every memory/linked-code cost.
  [Uniform-only qualification](rank-completion-qualification.json) remains the historical comparison cohort.


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
- **Ordered scalar maps:** an unequal-header Vector Binary/Call can seed the same demand at its own visited
  boundary, after existing activation and specialized error handling. The existing pure-map/Float proof resolves its
  vector edge. Invariant, boundary, source/scope grid and aggregate leaf/producer checks still apply. Unary/Paren
  only propagate inherited demand. Nested sum/scalar consumers of All can contain ordered children that select rows;
  admission therefore belongs to each eligible map, rather than only the root. There is no additional Task field
  or persistent node context. The [procedure and costs](README.md#mapped-group-contributors) distinguish added
  membership metadata from saved repeated label work.
- **Label projections:** ordered `label_join` and `label_replace` seed the same demand from their resolved vector
  child, including instant/equal-header consumers. Preserve label-argument preparation and each child's existing
  clock/ordinary-group/supported-leaf guards; label calls remain excluded from global numeric-map classification.
  Original ordinals and merged aliases select contributors without caching aggregate values or raw history.
  The [procedure and measured costs](README.md#label-projection-group-contributors) include complete cold/selected/
  next-clock failure controls, singleton/merged setup costs and the retained incomplete large baseline.
- **Completion:** a full child traversal must reach successful Done for the exact clock before membership filters
  later requests. Clear preparation credit before final delivery. Singleton group lookup is direct; nested requests
  use sorted/deduplicated accounted ordinal unions without changing arithmetic traversal order. RefCell borrows stay
  inside each operation and nested plans own different active unions.
- **Ownership and failures:** selected values follow ordinary acquisition and existing completed-window folds,
  except the separately admitted completed singleton Float reduction described below. Preserve cold nonmember
  failures, genuinely read warm selected failures and provisional output cleanup.
  Exact-key changes force full traversal. Contributor membership itself owns no raw-history array, child labels
  or aggregate payload; completed singleton Float ownership has its separate bounded owner below.
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
  reading its values again. The requested pair follows ordinary traversal; an admitted completed singleton
  Float operand may use the separate bounded result owner below.
- **Completion:** a processed prefix does not prove an unvisited suffix or full Done. If a fresh target becomes
  suppressed, continue ordinary traversal and preserve later operand failures. Do not replace a prefix with direct
  singleton output; surrounding operators may reject a sample or drain additional input.
- **Memory and limits:** reuse the existing discovery bitmap and replay wrapper. Optional boxed metadata keeps
  the maximum replay-enum and evaluator-Step layout unchanged. This membership mechanism owns no raw samples,
  retained values or child labels. Completed singleton Float ownership is separate; no dependency or public change.
  Metadata restoration and prefix
  walks can still grow quadratically with series count; no linear-runtime or optimality claim follows.
- **Evidence:** the [procedure](README.md#wrapped-grouped-joins) separates complete original outcomes, declared
  ordering-policy qualification, callback/control phases, query-owned memory and paired timing measurements.

## Ordered binary discovery

- **Choice:** let an eligible visited non-root ordered range Binary request the existing pair candidate before
  input descriptors move or copy. After output/cache/clocks freeze, pending candidates receive the unchanged
  prefix certificate. Root/wrapper replay keeps its existing route. An All-consuming parent does not imply that
  each descendant consumes All; its ordered child can repeatedly select individual source rows.
- **Proof:** retain operator/cardinality, original uniqueness, Float-only single-source ownership, supported leaf,
  producer/source/scope grid, invariant/boundary, name/fill and complete pair/hash/frontier checks. Instant/equal
  headers and unsupported ordinary grouped operands retain fallback. Failed certification discards the candidate.
- **Progress:** preparation cannot earn final credit. Cold final discovery validates original operands and pair
  prefixes; warm selected operands retain ordinary traversal, with the bounded singleton Float exception below.
  Exact-key changes restart discovery. Existing suppressed/unknown
  suffix, duplicate/cardinality, error, control and output-abort phases remain. Outer time-dependent pointwise
  parameters evaluate freshly even when the inner join has reusable metadata.
- **Coordinates:** an actual bypassed Matrix body consumes the source bindings in `sources[descriptor.sources]`.
  OrderedRows maps the result to the final descriptor ordinal afterward. Inner failure calibration and full public
  output checks must use their respective coordinates.
- **Ownership and cost:** reuse existing metadata, accounting and paged cursors without a new Plan field, evaluator
  mode, raw-value history or public contract. Extra matching metadata can raise peak bytes while reducing replay
  reads and allocation traffic. Whole-query linear complexity, minimal peak RAM or universal speed does not follow.
- **Evidence:** [the procedure](README.md#ordered-binary-discovery) and its finite qualification retain complete
  causal outcomes before growth assertions, actual inner failures, fresh dynamic parameters, fallback guards,
  original/native C/foreign checks and every memory, linked-size and paired-timing cost.

## Local grouped join operands

- **Choice:** at an eligible non-root ordered Binary, resolve both operands through preprocessing and OR
  their membership with existing single-child contributor demand. Keep Task unchanged and let ordinary child
  visits clamp their own clocks. An All-consuming ancestor can still have an ordered singleton descendant.
- **Proof:** keep supported operator/cardinality/name/fill and local clock guards for contributor demand,
  including instant/equal headers. Each ordinary group needs its supported selector/window leaf. Final Pair
  admission retains its stronger header/name guards, original uniqueness, Float
  single-source outputs and full pair/hash/frontier plus producer/source/scope grid checks. Partial group metadata
  can survive a rejected sibling/final certificate; it does not certify a Pair or change fallback outcomes.
- **Progress:** successful full child Done grants group credit; original successful Pair prefixes grant separate
  credit. Preparation clears both. Selected members retain ordinary evaluation, except the admitted completed
  singleton Float result below; new clocks validate again. Window caches can
  suppress physical reads, so actual inner failure calibration includes mixed directions with a raw side. Source
  binding ordinals remain distinct from final output ordinals; outer aggregate callbacks cannot locate inner phases.
- **Ownership and cost:** existing accounted member/group/Pair metadata only. No new persistent field, evaluator
  mode, raw array, metadata provider, storage or public contract. Additional setup/peak bytes are real even when
  replay work and cumulative allocation drop. Histories stay paged in both dimensions; implicit caps stay disabled.
- **Numerics:** retain upstream tolerance for continuous values and exact discrete outcomes. Preserve all original
  strict-bit diagnostics, exact native before/current results and exact reused cross-target/profile/page results.
  A diagnostic continuous one-ULP original difference is not a strict-bit pass or a changed tolerance.
- **Evidence:** [the procedure](README.md#local-grouped-join-operands) and its finite report retain the meaningful
  unchanged growth failure, actual group/Pair controls, complete original/default C/foreign outcomes, all measured
  memory/linked costs and both primary/slower followup timing samples. Existing shared reducer/projection algorithms
  are unchanged; no unrelated historical reducer corpus is reattributed to this source.

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

## Borrowed quantile-result labels

Quantile emission borrows immutable GroupDescriptor labels for the prepared roster lifetime, using the existing
SampleLabels::aggregate constructor. Sorting/index snapshots retain their own lifetimes; refreshing a snapshot
leaves descriptor label addresses intact. Empty-valued labels keep the existing copied normalization, preserving
raw group/hash identity. The helper's existing final control check precedes publication.

Retained descriptors, public output and actual mutation materialize independent labels through the existing
owners. Whole-matrix and per-step name flags retain their existing precedence. Source paging, parameter and
error phases, sorting/interpolation arithmetic, sample leases and provider backing keep their existing contracts.

The [finite qualification](README.md#borrowed-quantile-result-labels) records actual owner/terminal controls,
complete original/order-policy/native/C/foreign outcomes and all resource/timing/linked costs. Two historical raw
membership differences per quantile corpus remain explicit. No public type, field, cache or operator workflow is
introduced; no arbitrary-input, live, concurrent, RSS, physical-device or MSRV-execution claim follows.

## Quantile group contributors

- **Choice:** aggregate QUANTILE seeds existing contributor demand from its resolved body. Keep parameter demand,
  Finish, primary role, Task shape and LIFO parameter-first order unchanged. The selection extension below shares this
  established demand. The local consumer repeats ordinal requests during scanning, sorting and interpolation in
  instant, equal-header and range queries; no outer unequal-range condition is needed.
- **Proof and ownership:** each child retains its own invariant/boundary/source/scope clamp and existing supported
  ordinary/raw/window/map proof. First full child Done, exact-key selected fresh members, changed-clock validation
  and final preparation clearing keep their current owners. No arithmetic, histogram, name, raw-history, metadata,
  storage/provider/public interface or implicit-limit policy changes.
- **Controls:** actual Engine outcomes precede growth assertions; actual inner-group failures exclude both cold
  and warm requested identities. Preserve the initial miscalibrated test beside the corrected native/foreign
  executions. The real C shape validates single-group plan/ownership/output/cleanup, while plural activation is
  qualified by Engine/inner-Machine fixtures. Logical C statistics and broad physical fixture scopes stay distinct.
- **Trade-off:** metadata can raise peak memory and tiny-query costs while reducing raw replay and cumulative
  allocation. The 16-group timing cases save about 25.2/8.6 MB allocation traffic with equal peaks, but expression
  qualification retains up to 880 extra peak bytes, all slower controls and all linked costs. There is no universal
  speed or total-memory bound. See the [finite procedure and evidence](README.md#quantile-group-contributors).

## Selection group contributors

- **Choice:** extend the existing numeric-aggregate body demand to TOPK/BOTTOMK/LIMITK/LIMIT_RATIO, leaving accepted
  QUANTILE demand unchanged. Keep parameter Visit false, body primary role, Finish, Task shape and LIFO order.
  A nested selection inside a scalar parameter independently requests its own grouped child.
- **Proof and ownership:** preserve each child's clock/invariant/boundary/leaf checks, complete cold Done, fresh
  selected members, exact-key invalidation and preparation clearing. A nonproductive or cached-empty selection may
  request no final body. No arithmetic, tie/hash/native/name, provider/storage/contexts/public ABI or cap changes.
- **Controls:** complete actual Engine outcomes precede growth assertions; actual inner-group faults distinguish
  both cold/warm identities, selected values and next clocks. Qualify parameter and nested parameter priority.
  Independently captured original child rows establish only the declared empty-alias order policy; injected
  controls never supply performance or physical observations. Raw original failures remain visible.
- **Trade-off:** extra metadata can raise tiny-query peaks and time while removing repeated raw reads/allocation.
  The finite panel includes reduced peaks, all positive costs, slower follow-ups and linked-size changes; no universal
  speed or total-memory bound follows. The actual C shape checks one inner group; plural activation belongs to
  Engine fixtures. See the [procedure and evidence](README.md#selection-group-contributors).

## Empty group owners

- **Choice:** finish empty contributor construction and final control, then omit its unused boxed owner. A full
  globally empty group union cannot filter members or certify a productive Pair; keep ordinary All evaluation.
- **Retained:** singleton/plural static selector provenance and exact clocks, full range unions, child/parameter/
  opposite-input errors, diagnostics and cleanup. Singleton proof remains necessary for productive nested joins.
- **Zero-only change boundary:** no inline marker, new field/type/cache, proof traversal or public change
  or implicit cap. Initially empty but later productive group unions keep their owner.
- **Evidence:** whole empty outcomes precede the old 280-byte ownership failure. Productive raw/rate singleton
  joins, later rows and real required-input faults qualify. Fresh originals need no new order policy; inherited
  raw failures remain separate. Rust gate reuse across C-only assertion calibration keeps actual source identity.
- **Trade-off:** nonempty construction pays an emptiness check; retain all normal/fallback/slower and linked
  costs. Accounted memory and fixture timings do not prove RSS bounds or universal speed. See the existing
  [procedure and finite evidence](README.md#empty-group-owners).

## Shared scheduling and group proof owner

- **Choice:** one tagged owner replaces disjoint scheduling/group fields. Keep the stronger scheduling marker
  separate from aggregate provenance; store singleton provenance inline and retain plural filtering ownership.
- **Construction:** ordinary aggregate nodes and certified raw/map/label schedule nodes are disjoint. Keep the
  contributor builder's final validation/control. A singleton filtering Plan always declined activation, so its
  completion/active allocation is unnecessary; its selector identity still certifies productive nested joins.
- **Retained:** exact clocks, complete range unions, plural membership/Done credit, fresh requested values,
  parameters/opposite inputs, diagnostics and cleanup. This representation change added no field, traversal,
  value cache or public change; the later bounded singleton result owner is described below.
- **Evidence:** actual node layouts shrink by eight bytes on AMD64, ARM64 and ARM32. Inactive singleton ownership
  removes 280 bytes on 64-bit and 216 bytes on ARM32. All native paired workspace peaks/traffic decrease; raw reads
  stay unchanged. Complete results, scheduling separation, actual C statistics and target scopes are measured.
- **Trade-off:** tagged getters change branch/code shape. Retain all tiny/normal/fallback/slower timing and linked
  costs rather than infer speed from smaller structs. The earlier empty-owner cohort remains historical evidence;
  singleton representation is now inline. See the
  [procedure and finite evidence](README.md#shared-scheduling-and-group-proof-owner).

## Completed singleton aggregate values

- **Choice:** keep one lazy accounted boxed table in Folds. Sorted expression-node slots hold one current exact
  ReplayKey, completed Float bits, actual name flag and independent sample lease; no metric/time value array.
- **Admission:** require existing globally one-group provenance and exact ordinary frame/producer/source/scope
  clocks, plus the actual pending singleton physical side of a plural `group_left`/`group_right`. Recognize only
  typed transparent Paren and same-node OrderedRows frames. Exclude own diagnostics, materialized/invariant/
  boundary frames, plural/native/absent outcomes and unrelated one-pass consumers. Delayed-name proof fallback stays.
- **Publication and lifetime:** actual child Done, reducer finish and reached annotations precede insertion.
  Reserve/shift a successful Float slot before terminal control, then publish with constant work. Lookup releases
  its borrow before label/delivery construction. Deliveries retain independent leases. Key mismatch, preparation,
  clear and retain_numeric discard the owner; alternating nodes do not evict each other or retain old clocks.
- **Contributor demand:** ordinary local Binary group demand now also covers instant/equal headers with existing
  clock/name/operator/cardinality guards. Pair-specific certification keeps its own stronger restrictions. Cold
  inputs, many-side values and outer parameters keep their original evaluation; only admitted completed singleton
  Float reductions have the narrow freshness exception described here.
- **Costs:** Folds grows eight header bytes on each target. First table costs 192 bytes on AMD64/ARM64 and 168 on
  ARM32; two slots use normal vector growth to capacity four. No raw Record, Frame or NodeRoster field is added.
  Finite queries reduce reads/allocation traffic while some add Workspace peak; retain every cost and slower sample.
- **Evidence:** the [procedure and qualification](README.md#completed-singleton-aggregate-values) cover actual
  read phases, complete bits/labels/clocks, controls, sample ownership, native/ARM/C scopes and all source attempts.
  No public/provider/contexts/storage/numerical/profile/dependency/cap change or linear/minimal-RAM claim follows.

## Known-empty derived histogram probes

- **Choice:** share one private Fetch predicate between ordinary Step and generic backing materialization:
  omit kind 2 only when the actual frozen selected Descriptor has kinds exactly 1. This is the completed
  output-kind union, not a predicted return type, raw metric type, source ordinal or singleton-head proof.
- **Phases:** Reader starts with Float. A histogram request follows an exhausted Float occurrence whose
  restricted body actually reached Done. Omit only that already-requested histogram body, then use normal
  Reader advancement with pending/current/occurrence untouched. Every Float acquisition, multiplicity and
  next-clock request remains. Controls still precede advancement and required reads retain their errors.
- **Backing:** copy the selected byte into the existing temporary BackingSource; use the identical predicate
  in generic materialization. Do not change scheduled_reader_pass or infer exhaustion from its mismatched
  samples. Actual mixed/native/unknown masks retain fallback. The case with 14 Floats and two native samples
  proves real generic backing with union 3 rather than merely evaluating an unbacked mixed quantile.
- **Ownership and cost:** add no Reader/Step field, heap owner, value timeline, metadata duplicate, provider,
  ABI, storage format or cap. BackingSource adds 8 temporary bytes on 64-bit targets, with unchanged containing
  Action/Step/Reader sizes; ARM32 stays unchanged. Actual C output/report ownership and all 8 lease balances
  remain. Fewer physical reads and allocations do not imply a smaller simultaneous peak or process RSS.
- **Evidence:** the [qualification](derived-kind-qualification.json) retains complete native/ARM results,
  causal old failure, required post-preparation read failures, real generic mixed backing, C/C++ logical and
  physical statistics, every timing sample/slower control and actual linked/layout costs. Its separate
  source phases preserve compiler/input identities. The final Python-only input extension refreshes fixtures
  and proves byte equality before reusing earlier code qualification; no old binary is reattributed.

## Borrowed ranking-result labels

- **Choice:** canonical TOPK/BOTTOMK and explicit LIMIT emission borrows the exact immutable first representative
  through the existing SampleLabels owner. Raw empty fields remain intact; fresh winner labels are not substituted.
- **Reason:** selection already retains that owner for the sample lifetime. Repeated copying adds byte work and
  allocation traffic without supplying independent retention at this boundary.
- **Consequences:** retain the explicit replacement-site control check, existing kind/head/error priority,
  canonical ordinal/flags/times, fresh winner reads and snapshot semantics. Actual mutation, first descriptor
  retention and public collection still materialize independently. No new representation, owner or cache.
- **Evidence:** [the ranking-result procedure](README.md#borrowed-ranking-result-labels) and
  [finite qualification](ranking-result-labels-qualification.json) retain complete identities, controls,
  C ownership/statistics, all resource costs and slower timings. Allocation traffic differs from tracked peak
  and RSS; these finite shared-host observations establish no universal performance guarantee.

## Borrowed reserved-label detection

- Detection stops after its first exact reserved-name match; the existing owned removal still visits every field.
- Keep first-position empty-name errors, initial/terminal control checks and transactional conversion before mutation.
- No-match scans remain exhaustive. Raw order, duplicate/opaque/later-empty names and both removal modes stay valid.
- The frozen source remains independent through interrupted copies and partially changed owned cleanup.
- The conformance README records complete ownership/cancellation, real Engine/C, unchanged-memory and paired
  early/last/no-op measurements. Original campaigns retain their actual historical attribution.

## Grouping helper inlining experiment

- Two precise inline(always) hints removed actual prepared-key calls in native fixture/default-C binaries.
- The fixed positive/negative panel showed small mixed timings with identical query-memory/work observations.
- Text and local caller-frame costs did not establish a useful overall throughput or query-memory gain.
- Reject the measured candidate and retain the original source plus compiler discretion; no algorithm changes.
- The conformance README/report preserve all slower results, compiled costs and the corrected call-target check.


## Completed ranking delivery

TOPK/BOTTOMK shares the existing sort unchanged-selector/same-grid proof, then verifies Float single-head
inputs and a unique canonical-output-to-input mapping with at least two outputs. Every Some input binding
must agree with its descriptor/source; never-winning None inputs remain part of selection. Input kinds,
heads, catalog order, source, canonical ordinal, representative and hash retain their checks. One Plan
eligibility flag and one snapshot-owned
AtomicBool admit only warm canonical singleton delivery after checked full Done. Preparation clears completion,
new exact keys start false, and an old Step credits only its own held snapshot. Machine supplies a transient
target only for `RowFilter::Ordinals([canonical])`; existing cursor bounds skip completed nonselected delivery.
The requested winner still follows fresh acquisition and existing head/type/label/replacement checks.

There is no additional value array, cache expansion or Step/Frame/NodeRoster field. Native Plan/Snapshot/Step
types and accounted requests stay unchanged; actual target layouts, linked costs and all resource changes are
recorded. Cold suffix and next-clock validation, aliases/native/derived/shifted/plural routes and scalar prefix
truncation retain ordinary behavior. Heap metadata scanning remains, so fewer raw reads do not prove linear work.

The original full-input102 panel finds median ABS time ratios 0.43250/0.43762 for release/release-min and lower allocation traffic
with unchanged tracked peak in the 32-series wide TOPK example. A scalar fallback remains 6.5% slower in its fixed
release-min follow-up; every slower primary/follow-up and linked cost remains visible. Qualification preserves
the 216 existing strict upstream failures and their separate ordering-policy attribution. See
[completed ranking delivery](README.md#completed-ranking-delivery) for the mechanism, controls, measurements,
commands and exact source/runtime report. These finite results do not establish universal compatibility or speed.


The proper-subset extension retains all these completion rules. A changing union can reach cold full Done
without public output when its first canonical row is absent; its first public result may therefore be warm.
New actual Machine/C controls distinguish that phase from before-Done cancellation. No runtime field or value
array is added. All four measured target layouts equal accepted102. The subset panel finds grouped ABS median
ratios 0.96919/0.94395 for release/release-min, with mixed global/control results and all costs retained; no
universal speedup is claimed. See [proper ranking winner subsets](README.md#proper-ranking-winner-subsets)
and its separate exact source/runtime report. The earlier full-input measurements retain102 attribution.


Completed ranking delivery now borrows the frozen node source table transiently at its singleton
Machine call. Inside the existing warm certificate, resolve canonical descriptor -> sole original
input -> Binding.group, then check membership in only that heap. Preserve forward-only row/group/
position progress, target zero, pending replies and held-snapshot completion. Missing mapping retains
the canonical target and ordinary scan. Cold selection/validation, full Done, exact keys, reset and
fresh selected values are unchanged. No persistent field/array/cache is added; charge the actual
mapping attempt. Static argument/entry/code costs and all slower timing controls are separate from
tracked allocation layouts. See [direct completed ranking-group lookup](README.md#direct-completed-ranking-group-lookup)
and its separate accepted103/current report. The earlier102/103measurements keep their original scope.


## Certified derived Float completion

Generic derived Step and backed materialization share a visit-first Reader handoff. Ordinary matching
producer/source/scope grids, unique direct ordinals or single-source canonical rows, Float-only kinds,
nonmultiple heads and checked first-window interior geometry certify that Float1 will be empty.
Consume Float0 through the real visitor before continuing that same child through Done; leave the
expected empty occurrence pending. Unexpected post-mutation Action is an internal error. Preserve
required visit/write-before-tail failure priority, child options and sample credit. Endpoint/discarded,
later/disjoint, fixed/unchecked, native/mixed/multiple and scheduled paths retain their algorithms.
No Reader/Frame/history/value/completion cache is added. Input's small private source-mode enum and
BackingSource's proof have explicit per-target layout costs; no new metadata provider is introduced.
The [owning procedure](README.md#certified-derived-float-completion) distinguishes real Writer flush,
simulated Reader tail, calibrated Machine faults, actual full outcomes and every weaker/slower cost.

## Generated-label lookup indexes

COUNT_VALUES retains its original linear lookup and copy/push/polling through eight unique entries.
At the ninth, locally build the existing compact PositionIndex, polling/accounting prior-key visits and
rehash. Collection identity remains the grouping hash only; binding identity remains exact ordered raw
labels within that digest. First labels, wrapping counts, delimiter aliases and literal-zero hash behavior
are unchanged. Lookup metadata never orders results. Reserve the owner vector before index publication,
then immediately append before any fallible terminal poll. Failed construction retires local allocations;
post-append cancellation leaves consistent keys. Allocation capacity need not roll back.

Cache owns its binding vector and optional tracked index together. Plan.finish moves the whole owner;
Plan.project changes only existing canonical fields, preserving indexed raw keys. Step drops collection
lookup before sorting. Reset/format/overwrite/full modified-label validation/filter still precede every hit;
canonical label substitution remains after raw binding resolution. No sample/history array, per-binding
cached hash, shared index API or external metadata provider is added. All implicit caps remain disabled.

The first measured candidate allocated on its second entry and added insertion polls. The retained
refinement restores tiny work/polling, with measured512-series direct ratios0.37465/0.35953 and nested
0.27099/0.27189. Native private owners grow8bytes; lookup Workspace grows up to14608bytes in the panel.
Tiny/ordinary timing and linked costs remain explicit, including a5.18%slower projection follow-up.
The [owning procedure](README.md#generated-label-lookup-indexes) records both candidates, actual allocated
interruption/cleanup, distinct C activation scopes and the finite original/target/profile attribution.

## Conditional COUNT_VALUES preparation

- **Choice:** construct the local preparation `Option<RefCell<Cache>>` lazily using the existing exact-node
  COUNT_VALUES predicate. Borrow `Preparation` through the option; move the complete cache into its Plan only
  after the replay base drops. Keep actual empty/nested caches and exact-node filtering.
- **Reason:** ordinary nodes cannot use that local cache. Its empty vector owns a zero-byte accounting lease,
  so unconditional setup performs query-reference operations without retained payload.
- **Consequence:** no new persistent state, AST pass, polling, provider or ABI change. Error phases, full caches
  and all nonelapsed observations remain unchanged. Compiled C-provider paths prove the bypass; mixed timing
  and linked costs are explicit. Smaller entry frames do not prove runtime stack high-water or lower heap.
- **Evidence:** [finite qualification](conditional-count-preparation-qualification.json) and
  [procedure and costs](README.md#conditional-count_values-preparation).


## Borrowed COUNT_VALUES input labels

- **Choice:** use the existing local SampleLabels ownership state. Strictly ordered reset-surviving borrowed
  names admit virtual destination replacement/full validation,then checked copying of retained originals and
  setting the retained generated value. Owned or unordered/duplicate input follows its original transfer path.
- **Reason:** copying a borrowed field before grouping discards it spends allocation and byte work without
  contributing to the result. Validate the full effective modified label set to preserve error behavior.
- **Consequence:** old overwritten values and reset-empty fields are absent from validation;discarded effective
  fields still validate. Formatting precedes encoding. Poll after final destination membership even if it is
  discarded;an empty key has no later hashing poll. Result keys remain independently owned before unchanged
  collection/binding logic. There is no persistent layout,ABI/provider,metadata or raw-history change.
- **Evidence:** [procedure and finite measured scope](README.md#borrowed-count_values-input-labels) and
  [qualification](count-values-borrowed-inputs-qualification.json). All small/retained-label and compiled costs
  remain explicit;the local acquisition guard is distinct from whole-query peak memory.

## COUNT_VALUES retained-key copying

The accepted borrowed-input path filters discarded fields while constructing its retained owned key before
collection lookup. A later investigation of deferred ownership, checked borrowed hashing and transient
selection bits did not meet its throughput retention condition; none of those variants is retained.
Allocation traffic improved, while tracked query peaks stayed equal. The existing copying, binding and
collection contracts remain. The conformance guide's
[rejected investigation](README.md#rejected-count_values-hit-ownership) owns the
candidate mechanisms, measured costs, validation limits and historical evidence.

## Histogram result label ownership

Frozen histogram emission still copies the selected binding labels into independent query-owned memory.
A private experiment with exact borrowed views preserved complete outcomes and ownership, and reduced
allocation acquisition, but did not improve the fixed target timing median in both native profiles.
All experimental code and tests were reversed; tracked whole-query peaks were unchanged. Mutable
preparation, downstream materialization, lookup/duplicate/control phases and accepted qualification remain.
The conformance guide's [rejected investigation](README.md#rejected-frozen-histogram-label-borrowing)
owns the measured trade-off, complete failed/slower evidence and candidate-specific scope.

## Completed no-reset float windows

Historical zero-reset design; the current [single-reset extension](#single-reset-correction-ownership)
reuses the existing predecessor slot and retains separate qualification.

- **Choice:** one private boolean certifies absence of a strict decrease between admitted consecutive floats
  during the existing eligible initial matrix probe. Require pristine Rate/Increase state and disabled start
  timestamps. Compare old `last` before `receive`; do not populate reset-pass `previous` or retain extra samples.
- **Completion:** clear credit on scan entry and publish only after successful reached-page validation,
  closure and terminal control, at least two floats and no histogram allocation. Populated histogram/stale
  allocation still discards tentative float state and consumes normally. No partial probe earns reuse.
- **Arithmetic:** preserve source subtraction before any chronological reset addition. Only the completed
  no-reset proof omits replay. Interior decreases, start/reset/overlap, raw/derived, statistics/buffer and
  uncertified bounds preserve their original path. Ordinary NaN and stale NaN keep distinct adjacency behavior.
- **Accounting:** actual skipped walks reduce useful/physical reads and cancellation-reference traffic.
  Keep logical participation, same-query C statistics, balanced references, terminal release and honest peaks.
  No raw history array, new provider/metadata/API contract or implicit cap is introduced.
- **Trade-off:** selected native timing medians improve by about57–58%; all measured query peaks stay equal.
  The private boolean fits the measured native debug layout; current optimized target sizes are reported
  separately from unmeasured foreign baselines. Small linked code growth and slower fallback/control medians
  remain explicit in the [finite report](completed-no-reset-rate-qualification.json), with all original guards.

## Completed instant rate float windows

Current Float endpoint-handle reuse is documented under
[reused instant-rate endpoint ownership](#reused-instant-rate-endpoint-ownership).

A rejected fallback-sharing experiment is recorded under
[rejected shared instant rate fallback](#rejected-shared-instant-rate-fallback).

- **Choice:** a local InstantStep scan collects its existing Float tails during the existing eligible initial
  matrix probe. Require pristine tails and preserve start fields; constructors, receive, finish, derived
  receive-driven evaluation and persistent layout remain unchanged.
- **Completion:** full reached-page validation, closure and terminal control precede useful-read credit.
  Pure-float input omits one consuming walk with the same N useful reads. Failed probes may own earlier
  provisional endpoints honestly without earning useful-read credit.
- **Histograms:** never copy native values provisionally. Any populated histogram allocation, including
  stale/interior history, clears tentative floats before original consumption with completed progress.
  Final tail kinds cannot establish Float-only history. Raw, derived and uncertified windows stay generic.
- **Accounting:** charge every provisional Float copy. Preserve logical participation, same-query C statistics,
  actual work/physical reads, balanced references and terminal release. No raw array, extra owned endpoint,
  provider/metadata/API change or implicit cap is introduced.
- **Tradeoff:** selected native float queries use about 33–34% less time, with no increased measured query
  peak. Native/mixed fallback costs, eight persistent mixed follow-ups per profile, all slower controls and
  linked code growth remain explicit in the [finite report](instant-rate-acquisition-qualification.json).
  This qualification establishes neither uniform speed nor lower process RSS.

## Rejected shared instant rate fallback

- **Experiment:** move the identical window/geometry/pristine-tail checks into ordinary guarded tuple
  dispatch and share the existing generic Step receiver for unproven Instant inputs. Probe, consuming
  fallback, terminal phases, numeric finish, derived receive and persistent fields stayed unchanged.
- **Result:** native default-C text shrank, but ARM32 fixture text grew 70,248 bytes and ARM64 grew 512.
  This failed the all-target nonincrease rule recorded before source changes. Semantic, resource and
  native timing successes could not override that gate.
- **Disposition:** reverse only the two own source chunks and verify all accepted source hashes.
  Keep the original `InstantStep::scan` fallback. No candidate production change is retained.
- **Evidence:** the [negative report](rejected-instant-fallback-qualification.json) distinguishes the
  candidate source/binaries from the restored accepted implementation and retains all slower controls.
- **Lesson:** compiler/code-generation effects differ by target. Native linked savings do not establish
  ARM savings; check the selected linked artifacts before broader qualification. Fixture costs do not
  by themselves measure production ARM C or whole-Agent footprint.

## Reused instant-rate endpoint ownership

- **Choice:** recycle only an oldest Float sample handle whose underlying QueryState matches the incoming
  account. Existing Strong allocation identity recognizes clones and excludes independent queries.
- **Failure order:** expire credit before shifting endpoints and the unchanged checked value copy. Restore
  one unit after success. On failure, the zero-credit temporary drops and the shifted newest remains.
  Different-query handles keep the original drop-and-create path.
- **Reason:** a long Float window retains two endpoints, yet the previous receiver cloned and dropped a
  query reference for every input. Reuse removes that traffic without another field or history buffer.
- **Preserved:** native/stale input, complete acquired-page validation, histogram fallback clearing, numeric
  finish, derived delegation, exact sample peaks and the independently owned output. Avoid a transient
  ValueParts construction when a reusable handle already exists.
- **Evidence:** 120 complete old/current observation checks precede the actual construction guard; expiry
  cancellation and query affinity supplement the existing broader owners. Selected native fixture query
  time improves about 4%, with exactly equal serialized nonelapsed accounting. All six linked growth checks
  pass. The [finite report](instant-owner-reuse-qualification.json) retains every cost and slower control.
- **Limits:** fewer handle constructions are not fewer heap allocations or lower query memory. No language,
  API, metadata, dbengine, profile, ownership policy or query-limit change follows. Existing acquisition
  tradeoffs remain; finite timing does not establish device, concurrent, live or whole-Agent behavior.

## Reused Last Float ownership

- **Choice:** reuse the existing selected Float sample owner when its query state matches the incoming account.
  The private one-unit invariant follows from construction, successful replacement and output transfer. No field,
  raw-history array, buffer, provider interface, numerical rule or implicit cap is added.
- **Failure and overlap:** keep the checked copy before selection mutation. On success, charge one→two, replace the
  old Float, then charge two→one at the former lease-release boundary. This preserves both initialized values,
  separate global +1/−1 transitions, saturation, lock-visible phases and post-copy control behavior. Native,
  initial and different-query values retain ordinary construction; First/Last output transfers its existing lease.
- **Evidence:** actual old-producer semantic completion precedes the two owner-count failures. Four new tests,
  120 exact old/current observation records, complete original/paged comparisons, native/ARM units and actual C/C++
  consumers preserve the affected behavior. A bulk native failure's partial allocation is source-traced; its general
  peak assertion is not an independent phase discriminator.
- **Measured trade-off:** selected fixed-query median time is about 7–8% lower in both native profiles, with unchanged
  query memory/work/reads/statistics. Five chosen linked artifacts shrink; the size-optimized default-C link grows
  200 bytes. Concrete receive text grows 158 / 135 bytes, with unchanged finish sizes. The corrected static
  inventory appends owner methods and closures missed by the initial spelling filter, preserving the original
  partial record and unchanged linked gates. Every slower control and all six linked costs remain explicit.
  These are finite fixture costs and timings, not universal throughput, RSS or physical-device guarantees.
- **Evidence record:** [Last owner qualification](last-owner-reuse-qualification.json), with its actual 369-input
  candidate and 368-input accepted baseline. The prior report-only addition and all historical identities remain
  explicit. Other predecessor owners have separate semantics and remain independent future opportunities.

## Rejected Changes/Resets predecessor reuse

- **Trial:** after original checked acquisition, transfer only an existing same-query one-unit Float predecessor
  lease, hold two units through infallible Float comparison and commit, then release the overlap. Keep native
  fallible comparisons and independently owned output unchanged. No persistent field or raw-history cache.
- **Outcome:** rejected at the predeclared ARM64 fixture-size gate: +24,076 combined section bytes versus an
  8,192-byte allowance. Four links were measured; the two size-optimized native links and timing were not run.
  Successful focused debug ownership checks establish no production throughput gain.
- **Retained state:** restore the exact accepted source and preserve the actual failed/successful test attempts,
  snapshots and compiled artifacts separately. No public contract, query-memory rule or implicit cap changes.
- **Evidence:** [rejected predecessor qualification](previous-owner-reuse-rejected-qualification.json). The
  diagnostic function-size changes extend outside the edited module; compiler causality remains unisolated.

## Unchanged ARM64 compilation discriminator

- **Measurement:** rebuild the accepted source with a fresh private target, recording relevant inherited compiler
  overrides, resolved tools and verbose commands. Retain the historical source and binary attribution separately.
- **Outcome:** exact binary/section/selected-body equality; all 847 query pairs and 575 upstream comparisons match.
  The current baseline reproduces. Historical uncaptured settings and the rejected trial's compiler cause remain
  unknown; no profile change or relaxed size gate follows.
- **Evidence:** [ARM64 baseline reproduction](arm64-baseline-reproduction-qualification.json). This is a finite
  measurement and workflow clarification, with no accepted engine change or claimed performance gain.

## Catalog validation and matching share a local view

- **Ownership:** build one checked label-reference vector, validate it and pass it to the existing matcher. End
  that local borrow before the expansion handles its boolean result. No cache, field or label-byte copy is added.
- **Errors and phases:** preserve storage-error conversion for construction/validation and direct evaluation-error
  conversion for matching. Keep validation before filtering, control checks, snapshots and diagnostic order.
- **Measured trade-off:** lower allocation traffic and about 36% less evaluation time on selected metadata-heavy
  fixtures; no measured tracked-peak reduction. All six compiled-cost gates pass, including ARM64 growth of 8,076
  bytes. This is a local optimization with unchanged language, provider, contexts and paged-storage contracts.
- **Evidence:** [catalog view qualification](catalog-validated-view-reuse-qualification.json) retains actual old/new
  producers, phase controls, full query/original comparisons, C/C++, ARM, native and static checks with their limits.

## Rejected zero accounting-update elimination

- **Trial:** return for amount zero inside the private additive counter helper. Preserve public work hooks,
  all control checks, page counting, positive locks and saturation. No field, cache or raw-data change.
- **Evidence:** complete focused observations and actual default-linked zero-return routes pass. All six size
  comparisons pass, but primary selected timing ratios 1.0301/0.9968 miss the fixed 0.98 benefit requirement.
- **Retained state:** reverse only the owned edits after all writers finish; every accepted input matches the
  signed base. Preserve timings, slower controls, compiled costs and unrun scope without changing the gate.
- **Report:** [rejected zero-update qualification](zero-counter-update-rejected-qualification.json). No accepted
  speed or query-memory improvement follows from smaller compiled sections or a shorter zero route alone.

## Public direct default-hour evidence

- **Profiles:** execute two fresh default-profile originals with the retained checked Go executable. Preserve the
  four historical upstream-test outcomes separately; optional-profile evidence is not relabelled as default.
- **Replay:** a pinned public fixture and pure packet/check adapter reuse the canonical complete-result validators.
  Six cases pass on both native profiles and emulated ARM32/ARM64; a changed complete result fails the real checker.
- **Attribution:** preserve the historical catalog, 42-case default roster and actual compiled binary sources.
  The new evidence does not change calendar evaluation or establish additional language, speed or memory coverage.
- **Evidence:** [public hour qualification](public-hour-qualification.json) binds the actual original and current
  executions, zero sample/page reads, public replay inputs and deliberately incorrect result control.

## Set-aside inherited ranking completion experiment

- **Trial:** prove certified singleton logical Vector demand through transparent same-key frames while preserving
  physical Matrix modes and the original full Done completion flag. No partial credit, new cache or Step growth.
- **Measured benefit:** selected scalar-map timing ratios 0.3980/0.4049; control-family and six linked-cost gates pass.
- **Failed criterion:** 12 tested read-error scenarios per profile acquire and release 4,434 extra Workspace bytes,
  with unchanged retained/peak values and complete outcomes. The frozen traffic policy does not permit that cost.
- **Retained state:** all 377 accepted inputs restored exactly; candidate sources and evidence preserved. The policy
  exception remains unanswered. Continuing from the accepted engine does not imply a user vote or an accepted gain.
- **Report:** [complete set-aside qualification](ranking-parent-completion-set-aside-qualification.json). A diagnostic
  allocation sequence alone does not establish typed ownership, a universal memory bound or a no-cost repair.


## Ranking index capacity hints

The private ranking heap bounds tracked index growth with its already-resolved `k`, using the existing
`AccountedVec::push_with_capacity_hint`. Required length remains authoritative; growth is geometric and
only the final capacity is shortened. Zero `k` allocates no index buffer. There is no eager allocation,
new cache/field/history array or query limit. Shared accounting, replacement-overlap leases, insertion
order, fresh-value requests, snapshots and all provider/error phases remain unchanged.

TOPK/BOTTOMK and optional LIMITK/LIMIT_RATIO share this insertion. LIMIT_RATIO's `k` is the prepared
input count, rather than its fractional parameter. The capacity qualification separately proves smaller
live snapshot ownership and measures complete-query peaks/traffic. It preserves complete original outcomes,
all other exported resource fields and every slower timing; requested bytes are not an RSS contract.
See [the finite report](ranking-buffer-capacity-qualification.json) and
[the procedure](README.md#bounded-ranking-buffer-capacity).


## Set-aside cold ranking routing experiment

- Cold canonical-row group lookup was implemented without new persistent fields or arrays, preserving fresh
  samples, full cold delivery, nonwinner rows, ordinary fallbacks and held-snapshot ownership.
- Local emission-work gains did not satisfy the frozen complete-query criteria. Nested ABS benefited; bare Matrix
  delivery remained intentionally ordinary. Size-profile small-query costs also exceeded the original allowance.
- Requiring a bare-query work gain while preserving its unchanged Matrix admission was a planning conflict, not
  permission to broaden admission. The implementation, added test and factory were restored exactly to accepted122.
- Preserve the negative measurements and original criteria. Do not infer complete-query value from a local unit's
  growth bound. Existing warm routing, storage paging, completion credit and numerical/public contracts remain.
- See [the experiment report](cold-ranking-routing-set-aside-qualification.json) and
  [its measured scope](README.md#set-aside-cold-ranking-routing).


## Set-aside incoming ranking-value experiment

- One validated Float can serve all initial heap-insertion ancestor comparisons through existing scratch and its
  exact one-unit query lease. It must supply the comparison directly, leaving the skipped value/owner slot unused.
- The tested boundary was initial certified Push only. Root clears before Fix, which can compare two children;
  retaining another candidate there could add a third live value. Sort and final emission remain fresh.
- The candidate preserves observed results and owners and reduces complete-query reads/work/allocation traffic.
  It nevertheless fails the original small-query timing families and the size profile's targeted gain requirement.
- Reject the candidate and restore runtime, added test and factory exactly. Do not replace failed primary criteria
  with follow-ups or infer instruction-level causes from observed timing alone.
- See [the measured record](ranking-push-value-set-aside-qualification.json) and
  [its qualification scope](README.md#set-aside-incoming-ranking-value-reuse). No new field, sample array, cap,
  public contract, data format or metadata owner is retained.


## Set-aside first ranking-item controller experiment

- The first accepted member of a group needs no heap comparison. A private experiment omitted only the
  `heap_push(1)` Step/Done controller after successful append, preserving group/Scan state and sample expiry.
- Causal delivery, control and ownership checks pass. Complete-query work falls with exactly unchanged
  physical acquisition and serialized byte/reference/backing observations.
- The original complete-query throughput gates fail. Runtime, causal/helper tests, factory and test-only C
  observer are restored exactly; the optimization is not retained. Fewer transitions do not establish faster queries.
- See [the measured record](ranking-first-item-set-aside-qualification.json) and
  [its scope](README.md#set-aside-first-ranking-item-controller). Follow-ups do not replace failed primary gates;
  no new field, raw array, cap, public contract, data format or metadata owner is retained.


## Set-aside histogram constructor key borrowing

- A private experiment replaced temporary encoded keys with row positions and cached canonical hashes.
  It retained raw selected-field identity, complete validation, direct lookup through two groups, promotion
  order and separate classic/native passes. Construction-only metadata retired before plan publication.
- Complete outcomes, reads and ownership agree. Constructor and whole-query allocation traffic decrease;
  every measured whole-query memory peak stays equal.
- Original complete-query throughput gates fail. Runtime, shared helper, tests and factory are restored
  exactly; borrowing is not retained. Fewer temporary copies do not establish faster complete queries.
- See [the measured record](histogram-constructor-borrowing-set-aside-qualification.json) and
  [its scope](README.md#set-aside-histogram-constructor-key-borrowing). No new owner, field, raw array,
  public/data contract or cap is retained; follow-ups do not replace primary gates.


## Set-aside histogram delivery lookup

- A private experiment sorted existing frozen nonplural active groups by numeric ID above four total groups.
  Checked direct/binary presence lookup and locally returned positions removed repeated group scans without
  adding a retained owner. Logical/plural enumeration, cache IDs and phase-publication points stayed unchanged.
- Complete debug semantics, request traces, cancellation, cache flags, sample/byte ownership and work-growth
  checks passed. Actual debug layouts agreed; complete-query gains were not measured.
- Actual release fixture/default-C linked sections grew 16,352/9,700 bytes, exceeding the frozen 8,192-byte
  ceiling each. Both implementation files were restored exactly. No lookup optimization is retained.
- See [the measured record](histogram-delivery-lookup-set-aside-qualification.json) and
  [its scope](README.md#set-aside-histogram-delivery-lookup). No changed public/storage/provider/metadata contract.


## Set-aside shared histogram group ordering

- A private experiment shared the existing enumeration comparator with a numeric-mode boolean, enabled only
  for frozen nonplural populations above four. Logical/plural/tiny label/drop ordering stayed ordinary.
  Checked direct/binary presence lookup and locally returned positions removed repeated group scans without
  adding a retained owner. Logical/plural enumeration, cache IDs and phase-publication points stayed unchanged.
- Complete debug semantics, request traces, cancellation, cache flags, sample/byte ownership and work-growth
  checks passed. Actual debug layouts agreed; complete-query gains were not measured.
- Actual release fixture linked sections grew 12,248 bytes, exceeding the frozen 8,192-byte ceiling. The
  default-C link grew 508 bytes and passed. Both implementation files were restored exactly; no variant retained.
- See [the measured record](histogram-shared-order-set-aside-qualification.json) and
  [its scope](README.md#set-aside-shared-histogram-group-ordering). No changed public/storage/provider/metadata contract.


## Set-aside typed histogram selection

- A private experiment used a two-variant transient selection type to carry native rows or active classic
  positions. Global binding identities remained separate. Shared numeric ordering and checked lookup removed
  repeated scans without adding a persistent owner or raw-history storage.
- All focused semantic/control checks and 288 complete-query resource pairs passed. All four default-feature
  C-consumer footprint gates passed. Fixture footprints were recorded separately under a prospective policy.
- The larger-case timing median improved by about 5.6% in `release` and 2.0% in `release-min`. The latter
  missed the frozen 3% requirement, so both implementation paths were restored. No runtime variant was retained.
- See [the measured record](histogram-typed-selection-set-aside-qualification.json) and
  [its scope](README.md#set-aside-typed-histogram-selection). Historical failures and public contracts remain unchanged.

## Validated Sum acquisition reuse

- **Choice:** extend the existing completed initial Avg acquisition admission to Sum. Reuse the same checked
  window geometry and publication path; keep Sum's zero-initialized compensated arithmetic and operation order.
- **Failure and ownership:** complete page validation, closure and terminal checks precede credit. Native/mixed
  allocations, including stale history, retain consuming validation; later native arithmetic keeps its phase.
  No raw-history cache, persistent field, provider/ABI or limit change follows.
- **Evidence:** the [finite qualification](sum-acquisition-qualification.json) and
  [measured scope](README.md#validated-sum-acquisition-reuse) distinguish complete outcomes, logical statistics,
  physical reads, memory and timing. SRE parents are produced by each candidate, not copied from Go.
- **Trade-off:** the original 8,192-byte C-consumer growth ceiling fails only on ARM64, whose compiled text grows
  8,256 bytes. Decision 65.A explicitly accepts that specific increase. Native profiles and ARM32 have zero
  combined text/data/BSS growth; measured query-memory peaks do not increase. This grants no general size allowance.

## Rejected Count acquisition reuse

Count could reuse the completed initial Sum/Avg acquisition without adding raw-history storage. A finite candidate
preserved results, progress, logical reads and cleanup, including native/mixed/stale membership. It halved measured
Count reads, but failed the frozen release-min Avg-control timing criterion. The diagnostic repeat did not replace
that failure. The candidate was restored; the accepted admission remains Sum/Avg only. See
[count-acquisition-investigation.json](count-acquisition-investigation.json) for exact evidence and limits.

## Rejected Count driver specialization

A shared const-generic fold body was tried to isolate Count from the numerical specialization. It preserved the
focused semantic and ownership checks but added 72,116 bytes to the native release C executable, exceeding the
frozen footprint criterion. A single source body does not guarantee a compact compiled result. This candidate was
restored; see [count-specialization-investigation.json](count-specialization-investigation.json). The previous
Count admission experiment remains separately rejected under its original timing criterion.

## Rejected Present acquisition helper

Presence can reuse the completed acquisition result under the existing window proof. When allocated histograms
require consuming replay, that completed pass determines both presence and progress; the existing result lease
must remain. A changing-provider regression demonstrated this distinction.

The tested small helper passed focused checks but grew the native release C executable by 34,852 bytes,
exceeding its frozen footprint criterion. It was restored. Source size alone does not establish emitted size;
see [present-acquisition-investigation.json](present-acquisition-investigation.json). Count experiments remain
separately rejected under their original criteria.

## Shared acquisition validation visitor

- **Choice:** use a module-private named no-op visitor generic only over the error type for the initial acquisition
  probe. Its compiler identity no longer includes the consuming callback type. The consuming path is unchanged.
- **Contract:** preserve all existing traversal, validation, error, cancellation, credit and ownership phases.
  No persistent state, allocation, provider/ABI, arithmetic or query-limit change is introduced.
- **Evidence:** the [qualification](acquisition-visitor-qualification.json) and
  [measured scope](README.md#shared-acquisition-validation-visitor) record emitted bodies, every section size,
  complete query/accounting equality, original comparisons and actual C callers. All frozen footprint/timing gates pass.
- **Limits:** this reduces compiled size on the measured builds. Timings and measured query-memory observations
  are roughly unchanged and exactly unchanged respectively. Present read reuse remains a rejected separate experiment.

## Shared Present acquisition

- **Choice:** eligible unanchored Present reuses completed acquisition and avoids the second float-window read.
  Both no-op callbacks use the existing named visitor directly; it is now crate-private for this caller.
- **Ownership and failure:** validation and terminal checks precede useful-read credit. Populated allocated
  histogram history keeps consuming validation and uses that pass's full acquisition, including membership and
  progress. Existing finalization owns output credit; global absence still visits every series.
- **Boundary:** reuse the existing checked window geometry. No new state, allocation, raw history, provider/ABI,
  arithmetic or limit change; other folds and unproven paths remain unchanged.
- **Evidence and trade-off:** the [qualification](shared-present-acquisition-qualification.json) and
  [measured scope](README.md#shared-present-acquisition) retain complete outcomes, physical reads, unchanged
  logical credits, nonincreasing memory, all section sizes and the slower individual Sum control. Growth is
  measured against the already smaller shared-visitor build. All original frozen gates pass without exceptions.

## Shared fold validation visitor

- **Choice:** the remaining production no-op Sum/Avg consume callback uses the existing named function item.
  Stateful callbacks and the independent reference-test closure remain unchanged.
- **Contract:** callback type identity changes; phases, controls, progress, credits, arithmetic and ownership do not.
- **Evidence:** [qualification](shared-fold-visitor-qualification.json) retains actual section savings, complete
  outcomes, exact observations, timing controls and finite native/ARM/C/original checks. No throughput claim.

## Count from completed acquisition units

- **Choice:** reuse completed sample units only for proven Float-only, unanchored Count windows. Strict emitted
  timestamp progression makes the existing count exact even when physical Seek revisits a sample.
- **Ownership:** complete validation and terminal checks precede useful-read credit. Allocated histogram history
  retains the existing consuming visitor and its actual count, progress, failures and reached credits. Output
  finalization retains its sample lease; no new fields, allocation, interface or limits are introduced.
- **Fallbacks:** preserve statistics, buffered, non-reduced, wider, direct and anchored paths; the Sum/Avg
  arithmetic probe remains unchanged. Native histogram weights are never interpreted as counts.
- **Evidence:** [qualification](count-acquisition-units-qualification.json) records exact outcomes, bounded
  histories, actual failure phases, lower measured reads/time, all compiled sizes and slower individual controls.

## Native histogram field-map contributor demand

- **Choice:** request existing contributor metadata for HistogramSum/Count/Avg at ordinary unequal range clocks.
  Keep binary replay certification and schedule admission unchanged. This initial checkpoint excluded StdVar/StdDev;
  the subsequent variance-map stage below extends only local demand.
- **Ownership:** reuse only completed same-key membership. Cold validation, preparation clearing, fresh selected
  values, next-clock checks and diagnostics stay with the existing owners. Raw histories remain paged.
- **Trade-off:** contributor membership adds accounted per-member/group metadata while removing repeated unrelated
  reads. The finite 128-group example adds 4,376 B to tracked Workspace peak and cuts allocation traffic by about 95%.
- **Evidence:** [qualification](native-group-map-contributors-qualification.json) retains full outputs, original
  diagnostics, actual C/ARM callers, all compiled costs and slower timings. No API, limits or language change.

## Native histogram variance-map contributor demand

- **Choice:** extend local demand to StdVar/StdDev using the same completed group-membership owner. Keep their
  bucket-dependent kernels, schedule exclusions and statistics-only decode exclusions unchanged.
- **Error phases:** child Done proves membership, not kernel success. Full Engine preparation can itself report a
  malformed bucket. Separate actual Engine tests and constructed post-Done Machine/propagation owners qualify
  those phases without pretending that constructed metadata proves Engine admission.
- **Cost:** contributor metadata increases the measured peak within its frozen allowance while removing unrelated
  raw reads and allocation traffic. All timing, control, memory and independent default-C size gates pass.
- **Evidence:** [qualification](variance-group-map-contributors-qualification.json) preserves exact original outcomes,
  cold/selected/next-clock errors, caller cleanup, slower individual timings and finite measurement limits.

## Completed scalar aggregate values

- **Choice:** extend the existing completed Float table to exact scalar()/MixedBinary consumers under a
  running nonstreaming plural OrderedRows range owner. Both operand orders retain scalar-first evaluation.
  Parentheses, ordinary clocks, resolved child identities and exact ReplayKeys must all agree.
- **Preparation:** local Demand distinguishes None, Contributors and ScalarResult. Existing recursive
  contributor seeds keep their meaning. ScalarResult crosses Paren/scalar and admitted literal-sided Binary edges, requests global
  ordinary parameterless aggregate provenance at Finish, and stops before the aggregate body.
- **Lifetime:** successful full reduction and diagnostics precede publication. Existing independent leases,
  replacement, preparation clearing and terminal checks remain authoritative. No new value-table representation,
  retained history, provider surface, arithmetic route or implicit limit is introduced.
- **Fallback:** one-pass/instant/equal/grouped/nested/fixed/unsupported-wrapper consumers and native or absent
  reduction outputs keep ordinary evaluation. Float NaNs and native-input COUNT may qualify as Float results.
- **Evidence:** [finite qualification](README.md#completed-scalar-aggregate-values) includes complete originals,
  actual cold/warm/new-clock failures, C logical statistics/cleanup, compiler layout, footprint, timing and
  memory costs. The prior grouped-join-only admission description records the preceding checkpoint.

## Rejected literal scalar arithmetic trial

- Existing scalar-result demand and completed Float ownership were experimentally extended through direct
  Number-sided Binary ancestors with exact pending-phase/child proof. Ordinary arithmetic remained unchanged.
- The native timing/memory panels passed, but ARM64 linked growth exceeded the frozen ceiling by 700 bytes.
  The implementation and coupled test changes were restored at that checkpoint. The later
  [literal scalar arithmetic qualification](#literal-scalar-arithmetic) records its accepted continuation.
- The [negative qualification](scalar-arithmetic-rejected-qualification.json) retains every measured cost and
  source binding. A pure structural Binary predicate was separately tested and rejected below, not treated as an
  exception to this trial's gate. No numerical, storage, API or accounting contract changed.

## Rejected structural scalar arithmetic trial

- A pure Binary operand/phase classifier preserved the intended cache admission while removing the previous
  helper's unnecessary preprocessing context. Its linked ARM64 C cost still exceeded the unchanged ceiling.
- This successor was rejected at the early size gate, before functional or performance qualification.
  Exact restoration left the scalar-only optimization unchanged at that checkpoint; both failed gates remain historical.
- The [negative record](scalar-structural-rejected-qualification.json) makes no throughput or memory claim.
  Source length and fixture size are insufficient evidence for shipping executable footprint.

## Rejected scalar arithmetic inline hint

- An ordinary inline hint did not fold known caller facts: the ARM64 code text remained byte-identical to the
  rejected structural variant. CPU workaround handling and the footprint ceiling were unchanged.
- The early size gate rejected this variant before runtime qualification. Exact restoration leaves the accepted
  engine unchanged; [retained evidence](scalar-inline-rejected-qualification.json) reports no new performance gain.
- This closes the tested helper representations without selecting further arbitrary layout changes.

## Rejected C output label capacity hint

- Known-length output descriptor hints preserved lazy check/work/allocation order in the source trial, but failed
  the early ARM64 linked-size gate. [Evidence](output-label-capacity-rejected-qualification.json) separates
  24,480 bytes of additional linker padding from the two 12-byte function increases.
- At that rejection, runtime allocation savings remained a prediction. The exact owned diff was reversed; no compiler workaround,
  acceptance threshold, public interface or accepted runtime behavior changed.

## Proportional compiled-footprint review

The user revised the footprint policy on 2026-10-21. Minor fixed executable growth must not stop a useful
improvement merely because it crosses an experimental absolute-byte threshold. The blanket 8,192-byte veto
is retired for subsequent qualification decisions; it is not replaced by another arbitrary hard cutoff.

- Report compiled code, data and BSS separately from measured runtime allocations and RSS. A linked test
  executable's size is neither the complete Netdata executable's size nor a per-query memory charge.
- Report the before/after sizes, absolute and percentage changes, target/profile, and measured benefit.
  Track cumulative growth against an identified baseline as well as the immediate predecessor.
- Continue through minor fixed growth without requesting another exception when the change provides useful
  coverage or a demonstrated performance or memory benefit and satisfies the other qualification requirements.
  Discuss material resource trade-offs, target constraints or regressions with the user.
- Evaluate runtime memory separately, including costs that scale with concurrent queries, series or samples.
  This policy does not relax correctness, ownership, paging, accounting, throughput or query-memory requirements.
  It changes no configured query limit or compiler/CPU-workaround option.
- Preserve historical failed gates and their actual source/binary measurements. A later policy change permits
  renewed qualification; it does not retroactively turn an incomplete or failed experiment into a passing one.

For the retained scalar arithmetic trial, the ARM64 C consumer's code + data + BSS grew from 6,871,886 to
6,880,778 bytes: 8,892 bytes, approximately 0.13%. The user accepts this fixed increase. Its remaining
qualification was still required before accepting the optimization and is now recorded in
[Literal scalar arithmetic](#literal-scalar-arithmetic). The earlier
[negative record](scalar-arithmetic-rejected-qualification.json) retains the measured target benefits,
individual slower cases and unfinished checks.

## Literal scalar arithmetic

- Shared `literal_scalar_child` recognizes typed Binary operands with exactly one immediate Number sibling.
  Preparation forwards existing scalar-result demand only to the other operand.
- Runtime requires the exact pending child: phase zero with both slots empty, or phase one with completed left
  and empty right. Syntax and slot presence establish literal identity without floating equality.
- The ancestor walk still requires scalar(), exact ordinary clocks/keys and actual plural ordered consumption.
  Every arithmetic continuation remains unchanged; only the completed aggregate value is cached.
- Parenthesized/unary/compound/data-dependent siblings and call-based min/max retain ordinary evaluation.
  Native/absent values remain uncached. No new state or buffer is added.
- Finite results, errors, callback phases and costs are recorded in
  [the qualification](README.md#literal-scalar-arithmetic). Historical scalar-only evidence retains its original scope.

## C output label descriptor capacity

- Both known-length C output-label loops reuse `AccountedVec::push_with_capacity_hint`; capacity is bounded by
  the final count only when another allocation is needed. Required growth and old/new allocation overlap remain accounted.
- Per-label cancellation and work precede the push. No eager allocation, copied label bytes, metadata owner,
  retained descriptor cache or output table change is introduced. Metadata END and query callback errors keep their semantics.
- Measured savings concern transient descriptor capacity and allocation traffic. Whole-query peaks can remain unchanged
  because other owned state dominates. Timing and cumulative fixed-binary costs remain separate measurements.
- [Finite qualification](README.md#c-output-label-descriptor-capacity) records actual C calls, all slower primary cases,
  their bounded follow-up, native/ARM ownership checks and precise retained-versus-current evidence attribution.

## Rejected scalar arithmetic ownership transfer

- ScalarBinary was tested with an existing input sample lease moved to its result after successful arithmetic.
  The experiment targeted mutex/reference bookkeeping, not heap memory. The missing-owner fallback followed frame release.
- Complete native pilot outputs and fixture non-time observations matched, but release-min scalar-family timing
  missed the benefit criterion fixed before implementation. Exact baseline runtime and tests were restored.
- [Retained negative qualification](README.md#rejected-scalar-arithmetic-ownership-transfer) records all samples,
  both profiles, unchanged memory, unrun broader checks and source attribution. No new ownership rule was adopted.


## Completed binary subquery clocks

- **Decision:** reuse a constant-size interval of actually completed clocks for direct, certified unique Float
  binary subqueries. Keep selected values and membership fresh; do not retain a metric-by-time matrix.
- **Admission:** bare selector operands, one-to-one ordinary arithmetic/comparison, no fill or delayed-name mode,
  and a real range consumer through only transparent parentheses/same-node ordering frames. Every child-domain
  field stays exact; only checked positive aligned clocks can extend contiguous credit.
- **Completion:** successful unrestricted binary Done earns credit. A stopped endpoint, failed/native/nonmonotone
  discovery or restricted read cannot extend it. Selected left and right are rediscovered in order even if left
  is absent; ordinary pair reacquisition, matching, filtering and arithmetic remain authoritative.
- **Ownership:** one accounted interval per eligible plan, plus accounted active-attempt state. No partial
  global membership or generic prefix publication on restricted execution. Frozen child plans may be consumed
  during ancestor preparation; preparation-to-final clearing, replacement and recertification invalidate credit.
- **Measured trade-off:** the fixed synthetic target family takes about 40–41% of baseline time. One 16-pair
  case fetches 66,432 rather than 529,664 entries with a 529 B lower workspace peak; four-pair cases add 144 B.
  One aggregate-ratio control remains 10.3% slower in the fixed follow-up, with unchanged reads/work and an
  unresolved cause; a separate CPU diagnostic is mixed. Acceptance follows the predeclared family gates.
  The report preserves all controls and compiled costs separately from runtime memory. These observations
  do not establish live-dbengine throughput or RSS.
- **Evidence:** [completed-clock qualification](derived-binary-clocks-qualification.json) and the corresponding
  [reproduction procedure](README.md#completed-clocks-in-binary-subqueries). Initial non-activation, mismatched
  original options and strict order differences remain historical evidence, not silently converted passes.


## Unchanged provider resource snapshots

- **Decision:** skip ledger mutation for an entirely unchanged resource batch, after same-query validation.
  Borrow the first lease's live query owner for validation; clone it only for the existing changed-batch body.
- **Preserved:** all provider callbacks and hierarchy snapshots, ownership rejection, changed-batch locks,
  release-before-acquire transfers, exact memory counters and existing control/error phases. No new state or cap.
- **Measured result:** target-family time falls about 14% in the fixed synthetic panel; every reported nonelapsed
  observation matches. Preserve individual slower measurements and their single follow-up; no universal or live
  throughput claim follows. Compiled costs are separate from runtime memory.
- **Evidence:** [qualification](unchanged-resource-snapshots-qualification.json) and
  [procedure](README.md#unchanged-provider-resource-snapshots). This is separate from the rejected generic zero-count
  experiment; `MemoryLease::resize` and provider callback frequency remain unchanged.


## Completed grouped subquery clocks

- **Choice:** reuse the existing checked clock interval for direct Float-output SUM(rate) subqueries, with existing
  grouping membership. Selected contributors always read fresh values and run ordinary arithmetic.
- **Completion:** full child exhaustion plus selected-group finalization; no claim about unselected groups.
  Truncation, failure, own diagnostic preparation and preparation-to-final transitions cannot retain new credit.
- **Kinds and diagnostics:** emitted Float-only is not a raw-history certificate. Complete rate preparation owns
  mixed-window warnings, including absent nonmembers; native outputs retain fallback.
- **Memory:** lazy accounted constant-size interval, existing group/member metadata, no raw-history or value cache.
  Existing contributor plans carry the optional fields even when this optimization is not admitted.
- **Trade-off:** about 64% lower target-family time in the finite panel, with 512–816 B extra target workspace.
  Existing two-plan controls add 48 B; longer tested time extents keep workspace flat. Compiled costs and limits
  remain separate in the [qualification](grouped-subquery-clocks-qualification.json) and
  [procedure](README.md#completed-clocks-in-grouped-subqueries).


## Grouped rate-family admission

- **Choice:** admit only direct increase and delta beside rate in the existing grouped SUM subquery path.
  Keep the same Float-output proof, clock identity, fresh contributors and actual completion owner.
- **Arithmetic:** retain distinct kernels, including negative-gauge delta and start-aware counter increase.
  Shared storage traversal does not authorize arithmetic rewriting.
- **Memory:** no added fields or values; newly admitted paths activate existing bounded metadata.
- **Validation:** explicit enabled-start profiles, complete baseline and pinned-original outcomes, required-read
  failures and actual C consumers. Source, timing and proportional executable costs are in the
  [qualification](grouped-rate-family-qualification.json).


## Composed grouped ratio clocks

- **Choice:** compose existing grouped rate-family and unique pair intervals for direct one-to-one DIV subqueries.
  Reuse a shared static operand predicate and existing proven contributor plans; do not add a numerical cache.
- **Completion:** group exhaustion/finalization and binary Done remain independent. Exact resolved edges and clocks
  permit one binary parent crossing for group consumers; binary consumers still require direct range ancestry.
- **Values and diagnostics:** both operands execute fresh ordinary kernels. Complete preparation retains mixed and
  absent-series diagnostics; unsupported shapes keep fallback. Preparation-to-final clearing removes both credits.
- **Costs:** existing metadata becomes active without new fields. The measured larger case reduces both reads and
  workspace; the smaller case adds 344 B. Finite timings, per-target executable costs, original-order differences and
  all slower controls remain in the [qualification](grouped-ratio-clocks-qualification.json) and
  [procedure](README.md#completed-clocks-in-grouped-ratio-subqueries).


## Completed classic histogram discovery

A consumed literal histogram_quantile over direct SUM(rate/increase/delta) may compose existing child
completed-clock proof with frozen classic bucket metadata. Input proof is computed once and is independent
of ordered output certification: nonempty Float-only rows, single heads, strictly increasing input ordinals
and parsed bound/group membership. A borrowed filter carries the classic group and enumerates bucket-output
ordinals; the existing SUM union expands raw contributors. Equal numeric bounds with distinct text remain
distinct inputs. No new membership array, value cache or histogram completion owner is introduced.

Only the exact admitted Histogram vector edge permits one ancestry crossing, with unchanged ReplayKey checks.
The first cold discovery and histogram's own diagnostic preparation remain complete. Later selected discovery
requires an already completed child clock; ordinary bucket-count reads remain fresh. Child completion cannot
certify histogram arithmetic, disappearance is decided from current values, and preparation progress clears
before final delivery. Unsupported shapes retain fallback. The existing binary-parent route is preserved.
See [the finite qualification](histogram-discovery-qualification.json) for actual layouts, callback failures,
changing membership, special literals, complete original comparisons and proportional runtime/compiled costs.


## Completed grouped average clocks

- **Choice:** extend the existing grouped rate-family admission from SUM to SUM/AVG, including supported ratio
  and classic histogram compositions. Require the same nonempty Float-only, single-head child output proof.
- **Arithmetic:** recreate ordinary selected-group state; count only currently emitted contributors and preserve
  their order, compensated sum, overflow-to-incremental-mean transition and final division. Do not cache averages.
- **Completion:** keep child exhaustion and selected-group finalization as the owner. Binary pair and histogram
  completion remain separate; preparation credit clears before final delivery. Native/delayed/clock fallbacks stay.
- **Memory:** activate existing accounted membership and lazy intervals without adding fields or raw/value arrays.
  The larger measured direct case adds 816 bytes while reducing fetched entries by about 87%; its ratio reduces
  both entries and workspace. Longer tested time extents keep workspace fixed.
- **Evidence:** unequal and changing populations, mixed reducers, overflow, required failures and actual C callers
  are covered by the [qualification](grouped-average-clocks-qualification.json). Finite native timing and compiled
  costs remain distinct from live-provider throughput and process RSS.


## Completed ordinary grouped clocks

- **Choice:** use the existing ordinary-aggregation predicate for grouped rate/increase/delta admission. Retain
  the same Float-only, single-head child proof and all clock/parent checks, including ratio and histogram paths.
- **Arithmetic:** fresh groups preserve current membership and source order. COUNT/GROUP visit all required
  children; MIN/MAX preserve zero/NaN choices and sample ownership; STDVAR/STDDEV keep ordered online updates.
- **Completion:** retain child exhaustion plus selected-group finalization. Pair/histogram completion stays
  separate, and preparation credit clears before final delivery. No saved aggregate values or new owner is added.
- **Memory:** activate existing accounted contributor/interval metadata; raw data remains paged across both axes.
  Measured extent peaks stay fixed, while series-dependent metadata and result storage retain their existing costs.
- **Evidence:** complete before/original comparisons, reducer-specific numerical cases, actual required-read
  failures, C callbacks and native/ARM execution are recorded in the
  [qualification](ordinary-grouped-clocks-qualification.json). Native synthetic timing is separate from live data.


## Completed grouped comparison clocks

- **Choice:** recognize one literal scalar comparison around an existing interval group or certified grouped ratio.
  Preparation propagates contributor demand through the exact vector edge; runtime admits the same live map edge.
- **Scope:** keep parenthesis, clock, invariant and matrix checks. Limit comparison crossings with local traversal
  state, independently of the existing group-to-ratio allowance. Raw pairs and histogram wrappers stay excluded.
- **Semantics:** retain fresh comparison values, source-ordinal slices, scalar-first order, names/duplicates and
  logical statistics. Filtering removes samples; boolean comparisons retain zero. No comparison result earns credit.
- **Ownership:** existing Group/Pair completion and final clearing remain authoritative. No new retained owner,
  mapping table or value cache; raw samples stay paged across time and series.
- **Evidence:** the [qualification](grouped-comparison-clocks-qualification.json) records complete before/original
  outcomes, causal failure/completion tests, C/ARM execution and distinct query-memory/compiled costs.


## Completed grouped arithmetic clocks

- **Choice:** extend the existing literal binary-vector proof to the seven ordinary arithmetic operators,
  preserving comparisons. Preparation and runtime use the same resolved edge and eligible group/ratio owner.
- **Scope:** keep the immediate-number requirement, one-wrapper restriction and all clock/boundary/fallback checks.
  Rename private comparison-specific predicates; retain historical comparison tests and reports.
- **Semantics:** leave arithmetic kernels, scalar-first/left order, source slices, metadata and diagnostics unchanged.
  Nonfinite arithmetic results remain samples. Rate producers already remove reserved labels before grouping.
- **Ownership:** retain separate Group/Pair completion and final clearing; no new fields, owners or value cache.
  Raw samples stay paged across time and series, with existing series-dependent metadata and output storage.
- **Evidence:** the [qualification](grouped-arithmetic-clocks-qualification.json) retains independent numeric bits,
  complete baseline/original outcomes, causal failure/completion checks, C/ARM execution and all measured costs.


## Rejected broad fixed-map admission

- **Outcome:** fixed Float/unary admission over both groups and ratios failed its frozen ratio-family speed target;
  all runtime/test changes were reversed. See the [negative report](grouped-fixed-maps-rejected-qualification.json).
- **Finding:** Float Vector consumption truncates a ratio before Pair Done. Group completion cannot replace Pair
  completion or authorize skipping its unfiltered discovery. Parenthesized Call arguments resolve identically.
- **Implication:** preserve evaluation modes and independent completion proofs. A separately measured admission
  refinement may retain useful group/unary cases without enabling ineffective Float-over-ratio bookkeeping.
- **Limits:** positive subfamily results and passing finite semantics do not override the failed frozen gate.


## Qualified fixed maps over completed grouped clocks

- **Choice:** share a resolved vector-edge recognizer across preparation/runtime. Keep original literal binaries;
  initially admit unary ADD/SUB and fixed Float calls only over a qualified Aggregate child.
- **Reason:** Float calls over ratios stop before Pair Done, so their unfiltered discovery cannot reuse Group credit.
  Both ratio spellings initially remained generic. The separate prefix extension below covers them.
  Unary preserves its existing Matrix completion behavior.
- **Ownership:** retain all producer, clock, boundary and single-wrapper checks; no new cache, field or raw array.
- **Evidence:** the [refined qualification](qualified-fixed-map-clocks-qualification.json) reports current outcomes,
  causal controls, finite speedups and all resource costs. The [broad trial](grouped-fixed-maps-rejected-qualification.json)
  remains rejected; its failed gate and source attribution are unchanged.


## Successful physical prefixes in grouped ratios

- **Choice:** reuse successful discovery and a common processed physical-row prefix for one fixed Float map over
  an already-qualified grouped DIV. Keep full Pair completion as a separate authority.
- **Reason:** Vector consumption can stop at one pair before Pair Done. Group completion alone cannot justify
  skipping other pairs. Prefix promotion therefore requires a contiguous successful sweep across every covered clock.
- **Ownership:** a lazy accounted constant-size certificate; fresh selected discovery/reacquisition; ordinary
  fallback for absent or invalid selected inputs; reset with preparation. No raw/value or clock-history cache.
- **Limits:** existing unique Float frontier, one-to-one DIV, no fills, one map and unchanged clock domain.
  Other operators and nested maps keep existing behavior.
- **Evidence:** [ratio-prefix qualification](grouped-ratio-prefix-qualification.json) separates clock discovery,
  common-prefix advancement, full Done, sparse suffix cancellation, actual provider faults and measured costs.


## Unary over a fixed Float group map

- **Choice:** extend completed grouped-clock reuse through exactly Unary → fixed Float → qualified Group.
- **Reason:** Unary consumes Matrix, so the inner Float can finish its Group. An outer Float or scalar binary
  consumes Vector and cannot use the same completion argument.
- **Ownership:** reuse existing Group completion; return immediate resolved edges and preserve every intermediate
  clock/boundary check. No new owner, raw/value cache or evaluation mode.
- **Limits:** literal parameters, actual qualified Group and exactly two maps; ratio producers and further wrappers
  retain their existing paths. Final endpoint truncation and preparation clearing remain unchanged.
- **Evidence:** [unary/Float group qualification](unary-float-group-clocks-qualification.json) records causal tests,
  complete outcomes, all slower/cost observations, source phases and native/ARM caller checks.


## Delivered histogram in an instant-rate probe

- **Choice:** stop provisional Float endpoint updates only after a histogram reaches the eligible probe callback.
- **Reason:** successful delivery proves the original consuming fallback will discard these tentative values.
  Allocation alone is insufficient because it includes stale and undelivered history.
- **Lifetime:** keep earlier endpoints until the established post-probe clearing point; continue validation and
  skipped-Float control polling. Native copies and final arithmetic remain in consumption.
- **Accounting:** count actual eliminated copy work honestly; failed-probe sample peaks can decrease. No new
  persistent owner, raw array, public interface or fixed query cap.
- **Evidence:** [mixed instant probe qualification](mixed-instant-probe-qualification.json) separates complete
  results, causal ownership/failure checks, native/ARM callers, query-memory observations and compiled costs.


## Completed grouped selector clocks

- **Choice:** extend the private ordinary-group recognizer to allow direct instant selectors only at consumed
  subquery demand and Group interval qualification. Four composition callers remain rate-only.
- **Proof:** delivered Float-only/single-head descriptors cover the actual child clock grid. Raw history can
  contain unselected histograms. Existing replay schedule and successful Group Done establish exact-key credit.
- **Ownership:** reuse existing grouping metadata; cold traversal remains complete and warm selected values
  are read fresh. No raw-value array, public contract, retained-data change or new query cap.
- **Cost:** measured Workspace grows 544B at four groups and 816B at sixteen groups while repeated reads and
  allocation traffic fall. Time-extent observations are at fixed cardinality, not a universal memory bound.
- **Evidence:** [grouped selector qualification](grouped-selector-clocks-qualification.json) separates strict
  and agreed-order results, actual C callback controls, native/ARM checks and compiled/runtime costs. Reference
  capture options must match requests; a fixed-lookback adapter does not qualify custom-lookback inputs.


## Completed literal-comparison set discovery

- **Choice:** admit one literal-scalar comparison over direct Selector/Paren operands only on multi-step grids.
  Two local traversal guards prevent extension into projection wrappers, chains or arbitrary calls; existing
  raw/projection/nested-set paths retain their checks. Shared fixed-map resolution owns scalar/type validation.
- **Proof:** original child uniqueness and final Float/single-head certification remain required. Both actual
  discovery streams must finish at the exact key before reuse. Warm selected comparisons execute freshly.
- **Diagnostics:** final Float output does not certify raw Float-only history. Full comparison preparation owns
  incompatible native annotations and transfers them before raw child descriptors are released.
- **Ownership:** reuse the existing current-key bitmap and serial Plan lifecycle. No new retained state or raw
  cache; preparation clears completion before final evaluation. False bool values remain present set members.
- **Cost:** target Workspace increases280B; a nested-set control increases560B. Read/time reductions and all
  fixed/cumulative compiled costs are measured separately. No new implicit query cap or universal memory claim.
- **Evidence:** [comparison-set qualification](comparison-set-discovery-qualification.json) separates complete
  baseline and original-policy checks, strict ordering differences, malformed-storage exclusions, actual callback
  phases and portable tests. Single-tick and other computed operand classes keep their existing fallbacks.


## Completed ordinary-window set discovery

- **Choice:** admit direct ordinary per-series Rate/Fold calls, optionally under one literal comparison, on
  multi-step grids. Reuse the existing direct-Matrix/Selector validator and common source/scope/producer clocks.
  Global absence, Numeric reducers and unsupported geometry retain their existing evaluation paths.
- **Proof:** both actual discovery streams complete at the exact key; original uniqueness and final Float,
  single-head descriptors remain mandatory. Zero bool outputs are present. Completed preparation owns native
  suppression diagnostics even when final descriptors contain only floats.
- **Ownership:** reuse the current membership bitmap and serial Plan lifecycle. Selected original expressions
  preserve existing window evaluation, including same-clock fold reuse. No new raw history or values cache.
  Direct label-wrapped windows remain excluded, while independently qualified nested set plans may compose.
- **Cost:** measured target Workspace increases280B; a nested-set control increases560B. Time/read reductions,
  all individual controls and proportional/cumulative compiled costs are reported separately, without new caps.
- **Evidence:** [window-set qualification](window-set-discovery-qualification.json) records complete baseline and
  original comparisons, strict ordering differences, calibrated failures, C read boundaries and portable checks.
  C tests must observe actual required reads: an output callback does not imply the next row must read storage.


## Completed clocks in nested grouped subqueries

- **Choice:** qualify exactly two ordinary aggregate levels under a direct consumed range, with an existing
  selector/rate/increase/delta terminal. Keep one-level map, ratio and histogram composition admission unchanged.
- **Preparation:** propagate grouped demand through the outer aggregate only for this shape. Require prepared
  inner interval support and outer Float/single-head descriptors; syntax alone cannot qualify the outer plan.
- **Completion:** preserve separate existing plans and child output ordinals. Runtime ancestry permits one
  collecting aggregate edge, requiring both prepared plans and exact clocks. Each level earns its own interval;
  completed inner work can coexist with an uncredited truncated outer endpoint. Failure and clearing retain
  their existing ownership, including ordinary selected evaluation and same-clock fold reuse.
- **Cost:** add no state fields or raw/value cache. Finite targets take about 64–69% less time; measured maximum
  Workspace increase is 1,248 bytes. Read/work, individual controls, fixed-cardinality extent and proportional/
  cumulative compiled sizes are recorded separately in [qualification](nested-grouped-clocks-qualification.json).
- **Boundary:** third-level and wrapped nested groups remain valid generic expressions. Their optimization is
  independent work, not omitted support or a reason to broaden the direct ancestry proof implicitly.


## Completed clocks through projected groups

- **Choice:** allow one nonmerging label_join/label_replace over one ordinary selector/rate/increase/delta
  group under a directly consumed subquery. Keep the existing projection and group owners.
- **Proof:** preparation preserves grouped demand only for this shape. Runtime requires equal binding/output
  counts, exact resolved child and clock ancestry, and an existing interval-capable group. Each output has a
  source; equal counts rule out merged aliases. A merged prepared plan does not earn runtime interval credit.
- **Ownership:** preserve label validation, duplicate detection, diagnostics, ordered source translation,
  actual completed traversal, ordinary selected evaluation and final clearing. Add no fields or raw/value cache.
- **Cost:** finite target families take about 59–68% less time, with measured maximum Workspace growth112bytes.
  Read/work, all category peaks, individual controls, extent and proportional/cumulative compiled costs remain
  separate in [qualification](projected-grouped-clocks-qualification.json).
- **Boundary:** broader compositions and merged aliases keep their valid generic evaluation. All-Float ordinary
  groups retain contiguous ordinals; sparse clock membership does not imply reordered whole-query bindings.


## Validated extrema acquisition reuse

- **Choice:** ordinary Min/Max joins the existing Sum/Avg acquisition gate, using the same window proof and
  original selection order. No new storage, state field or public contract is introduced.
- **Ownership:** retain the existing extremum earlier; complete validation, closure and controls precede
  publication and Float-only shortcut credit. Histogram allocation still requires consumption, which credits
  each reached sample. Late failures release provisional values.
- **Boundaries:** anchored/statistics/buffered/wider geometry and derived folds remain unchanged. Timestamp
  extrema and variance are separate opportunities, not unsupported expressions.
- **Evidence:** [qualification](extrema-acquisition-qualification.json) and
  [measured scope](README.md#validated-extrema-acquisition-reuse) retain exact comparisons, early-lease failures,
  38–45% target time reductions, unchanged measured category peaks and separate proportional compiled costs.


## Validated variance acquisition reuse

- **Choice:** ordinary Stdvar/Stddev join the established numeric acquisition gate under the same window proof.
  The existing scalar accumulator performs identical updates; final division and square root remain after acquisition.
- **Ownership:** no new persistent field or buffer. Complete validation, closure and controls precede publication
  and Float-only shortcut credit. Histogram allocation still requires consumption with per-sample credit;
  unsupported geometry keeps the existing path.
- **Evidence:** [qualification](variance-acquisition-qualification.json) and
  [measured scope](README.md#validated-variance-acquisition-reuse) retain original arithmetic edge cases,
  33–39% lower target query times, unchanged measured memory peaks and separate proportional compiled costs.


## Validated endpoint acquisition reuse

- **Choice:** First/Last join the existing ordinary acquisition gate; the existing selected value is retained earlier.
- **Ownership:** preserve histogram copying/normalization and mandatory consumption. Partial-copy and later-cursor
  failures release all owners; complete validation and controls precede results and Float-only shortcut credit.
  Required histogram consumption credits each reached sample. No new persistent state.
- **Names:** direct endpoint results retain the metric name, including mixed histories. Arithmetic consumers apply
  their normal name rules. Geometry fallbacks and derived folds retain their established paths.
- **Evidence:** [qualification](endpoint-acquisition-qualification.json) and
  [measured scope](README.md#validated-endpoint-acquisition-reuse) record 36–58% lower target times,
  unchanged measured peaks including wide histograms, and separate proportional compiled costs.


## Validated regression acquisition reuse

- **Choice:** ordinary Deriv/PredictLinear acquisition updates the existing scalar NumericStep provisionally.
  Validation, required histogram consumption and final controls still precede completion and publication.
- **Contracts:** retain exact compensated arithmetic, prediction evaluation time, mixed diagnostics, parameter
  initialization, instant Numeric caching and shared/derived/fallback scanning. No new persistent storage.
- **Evidence:** [finite qualification](regression-acquisition-qualification.json) and
  [measured scope](README.md#validated-regression-acquisition-reuse) separate timing, accounted peaks,
  actual failure/control witnesses, preserved test correction and proportional compiled costs.


## Validated Changes acquisition reuse

- **Choice:** admit ordinary Changes to the existing acquisition proof and one-predecessor accumulator.
  Complete validation, required native consumption and terminal controls precede results. Float-only reuse
  credits reads after successful acquisition; required histogram consumption credits each reached sample,
  preserving reached credits on later cancellation or failure.
- **Contracts:** preserve native equality, raw strict timestamps, derived merge errors and all fallback paths.
  Resets remains separate because its native operation can return additional errors during evaluation.
- **Cost:** earlier predecessor ownership can modestly increase native-buffer overlap. Retain actual peak
  changes, failed preparation records, complete reference comparisons and proportional compiled costs in the
  [finite report](changes-acquisition-qualification.json) and [measured scope](README.md#validated-changes-acquisition-reuse).


## Rejected Float Resets acquisition

- **Decision:** retain the accepted Resets path. A Float-prefix acquisition candidate preserved results and
  native error order but added repeated work on required native/mixed consumption.
- **Evidence:** [negative qualification](resets-acquisition-rejection.json) records useful Float savings,
  unchanged accounted peaks and persistent 8–23% individual mixed/stale regressions. Passing pooled controls
  did not override the explicit requirement to examine material native/mixed costs.
- **Disposition:** exact accepted runtime restored; failed candidate remains private evidence. No native
  error-phase relaxation or public contract change. Predecessor accounting is a separate future experiment.


## Same-query Float predecessor ownership

- Complete checked copying before taking any predecessor lease. Only a Float-to-Float replacement with a
  one-unit same-query owner transfers it; other paths construct ownership normally.
- Retain two units until the prior value is replaced, then return to one. Preserve saturation and output's
  independent owner. The transferred Float comparison is infallible; native comparisons retain their phases.
- This removes reference-count clone/drop operations without changing sample accounting updates or traversal.
  All 2,934 measured nonelapsed observation pairs agree; direct Float timings improve about 4–6 percent.
  Derived controls do not all improve. No new persistent fields, raw arrays, implicit limits or public contract.
- Use current proportional compiled-size review. The old stage 116 rejection and its then-fixed gate remain
  historical; they are not rewritten or treated as present performance evidence. See the current
  [qualification](previous-owner-reuse-qualification.json) and [measured scope](README.md#reused-float-predecessor-ownership).


## Single-reset correction ownership

- An eligible completed Float acquisition can certify zero or one decrease. Store the first reset predecessor
  in the existing Point slot; a second decrease clears it and permanently disqualifies that acquisition.
- Resize the existing sample lease at both transitions before `receive` can observe cancellation. Completed
  validation and terminal control still precede reuse. Native/stale-native allocation discards tentative state.
- Subtract last minus first before the single correction addition. Multiple resets retain chronological replay;
  start-enabled, Delta, raw/derived, statistics, buffered and uncertified paths stay unchanged.
- No field, raw history buffer, public option or new provider responsibility is introduced. Logical sample
  ownership may occur earlier on failed prefixes while the measured byte peaks remain unchanged. Record real
  read/work reductions; never equate a per-fold statistic with the total for a composed query.
- The [finite qualification](single-reset-rate-qualification.json) records current-source results, both timing
  runs, proportional compiled costs and test-harness corrections without replacing historical evidence.


## Instant Boolean rate descriptor ownership

- A completed ordinary Float Rate/Increase child has the same presence, kinds, already-dropped name and
  source ordinals as its literal Boolean parent. Reuse only at a root genuine instant request with matching
  source/scope/producer clocks and no invariant, boundary, ordered child or selector clock modifiers.
- Check each descriptor and terminal cancellation before taking completed output. Float-only descriptors
  must have no repeated/sorted heads or source list. Missing output falls back; completed empty output is valid.
- Transfer through ordinary roster output, preserving sparse ordinals and exact label ownership. Do not use
  projection bindings, copy labels or retain sample values. Keep child-order annotation merge, normal root
  statistics with zero local units, matrix preparation and normal child-output release.
- Final values are freshly evaluated through existing paged reads and arithmetic. Existing early/delayed
  duplicate phases and provisional output abort remain authoritative. Start-enabled folds still perform
  their required consuming passes; logical sample totals must not count removed preparation replay.
- Generic range (including equal endpoints), nested, dynamic, non-Boolean, modified-clock and native-output
  paths remain valid. The [finite qualification](boolean-rate-preparation-qualification.json) binds complete
  original results, causal lifecycle checks, all slower controls, memory observations and compiled costs.
