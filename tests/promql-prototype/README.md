# C PromQL experiment

Independent C11 parser/evaluator for the common dashboard/alert profile. It uses a request arena, explicit typed
values, native hash indexes, and structured error returns through the shared experimental C interface.

The shared interface, C caller, Go oracle, fixtures, runners and comparison live in the adjacent `netdata`
worktree under `tests/promql-prototype`. Run its `build.py c`, then its `run.py` after building all three candidates.
See that worktree's README and `evidence.json` for the tested scope, exact source digests and limitations.

Results own all returned labels/points, including temporary data. One release reclaims the request arena. The
trade-off is higher temporary retention and manual control-flow/ownership maintenance. This is a prototype,
with no production API, collector, health or Cloud integration.
