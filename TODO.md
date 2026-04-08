# TODO: Runtime Metrics Shutdown Test Split PR

## Purpose

Publish the unrelated framework test fix separately from the BGP work.

This PR covers only:
- `src/go/plugin/framework/functions/runtime_metrics_test.go`

This PR exists because the failure was discovered while repairing the mixed BGP PR, but the code change is not BGP-specific.

## Reference Source

- Mixed source worktree: `/home/costa/src/PRs/netdata-bgp-monitoring`
- Mixed source TODO: `/home/costa/src/PRs/netdata-bgp-monitoring/TODO-BGP-MONITORING.md`
- Relevant source note in that TODO:
  - the third local repair pass recorded that the remaining failure was a real test failure in `plugin/framework/functions`, not a docs or lint issue
- If review context is missing, start from the mixed worktree record before widening scope.

## TL;DR

- The test previously assumed runtime gauges drop to zero immediately at shutdown.
- This PR changes the test to wait for those gauges to settle before asserting zero, which matches the observed shutdown timing.

## Analysis

- This is a one-file test-only fix.
- The changed test is:
  - `TestManager_RuntimeMetricsScenarios`
- The added logic waits up to one second for these runtime gauges to settle after shutdown:
  - `invocations_active`
  - `invocations_awaiting_result`
  - `scheduler_pending`
- The fix does not change framework runtime behavior; it changes the test expectation from "immediate zero" to "eventual zero within a bounded wait".

## Decisions

1. Keep this fix separate from both `snmp-bgp` and `bgp-daemons`.
2. Do not mix any BGP files into this PR.
3. Treat this as a test-stability fix, not a product feature.

## Pending

- No known functional follow-up is captured for this split beyond normal review.
- If CI still exposes flakiness after this fix, the next investigation target is the runtime manager shutdown/metric publication ordering itself, not the BGP collector work.

## Plan

1. Publish this as a small standalone PR.
2. Make the PR body explicit that the failure was discovered during BGP PR repair, but the code path is unrelated.
3. Keep validation limited and direct.

## Implied Decisions

- Unrelated CI/test fixes should not be hidden inside large feature PRs.
- A test that races asynchronous shutdown should wait for the intended steady state instead of asserting on the first read.

## Testing Requirements

- `go test ./plugin/framework/functions -run TestManager_RuntimeMetricsScenarios -count=50 -race`

## Documentation Updates Required

- None.
