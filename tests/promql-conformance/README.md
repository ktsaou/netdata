# PromQL conformance

The generic frontend, evaluator, source-bound fixtures and C/C++ qualification belong to the
[contained Rust library](../../src/crates/promql-engine/README.md). Its complete source and conformance suite
are ordinary files in this Netdata branch. The independent Cargo workspace and C interface remain intact;
separate repository publication is postponed.

See its [conformance guide](../../src/crates/promql-engine/tests/promql-conformance/README.md),
[approved engine decisions](../../src/crates/promql-engine/tests/promql-conformance/DESIGN.md),
[import bindings](../../src/crates/promql-engine/IMPORT.json) and
[relocation qualification](../../src/crates/promql-engine/tests/promql-conformance/extraction-qualification.json).

Historical private captures under this directory's ignored `.tmp/` remain in place with their original
source and binary identities. The original experiment remains in `tests/promql-prototype/`. These
captures do not qualify a different source revision or live dbengine integration.

From the Netdata root, run generic tools through the library, for example:

```sh
python src/crates/promql-engine/tests/promql-conformance/build_frontend.py
python src/crates/promql-engine/tests/promql-conformance/c_embedding.py --profiles release release-min --abort-check
```

For commands shown inside the library's conformance guide, first change into `src/crates/promql-engine`;
its "repository root" means that independent Cargo root. Fixture tests use their own project-local scratch
directory. Existing private Netdata evidence remains separately retained and is not included in published source.
