# C++ PromQL experiment

Independent C++17 parser/evaluator for the common dashboard/alert profile. It uses owned AST nodes, standard
containers, RAII for intermediate/results, and exception containment behind the shared experimental C interface.

The shared interface, C caller, Go oracle, fixtures, runners and comparison live in the adjacent `netdata`
worktree under `tests/promql-prototype`. Run its `build.py cpp`, then its `run.py` after building all three candidates.
See that worktree's README and `evidence.json` for the tested scope, exact source digests and limitations.

Results own all returned labels/points. Allocation exceptions return a static emergency error that the release
function handles. Production allocator accounting and platform qualification remain untested. This is a
prototype, with no production API, collector, health or Cloud integration.
