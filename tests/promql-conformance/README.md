# PromQL conformance

The generic frontend, evaluator, source-bound fixtures and C/C++ qualification now belong to the
[pinned standalone engine](../../src/crates/promql-engine/README.md). Netdata consumes that repository
through an immutable Git submodule; there is one authoritative source and conformance suite.

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

The library's fixture tests use their own project-local scratch directory. Existing private Netdata
evidence is retained separately; it is not copied into the public library.
