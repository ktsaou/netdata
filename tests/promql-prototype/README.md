# Rust PromQL experiment

Independent Rust parser/evaluator for the common dashboard/alert profile. It uses owned AST/value types, standard
collections, and an explicit `repr(C)` input/result adapter. It has no external Rust dependencies or async runtime.

The shared interface, C caller, Go oracle, fixtures, runners and comparison live in the adjacent `netdata`
worktree under `tests/promql-prototype`. Run its `build.py rust`, then its `run.py` after building all three candidates.
See that worktree's README and `evidence.json` for the tested scope, exact source digests and limitations.

Results own all returned labels/points. The boundary catches unwinding panics; the injected-failure probe is
a controlled error after evaluation. Standard-library allocation failure can still abort, and is not proved
recoverable. This is a prototype, with no production API, collector, health or Cloud integration.
