# PromQL engine instructions

This directory is part of the Netdata repository. Follow the root [AGENTS.md](../../../AGENTS.md) and its
shared SOW lifecycle; use the active Netdata SOW for engine/integration changes. Retained standalone SOW
workspace state is private and does not define another source authority.

## Scope

The independent Cargo workspace owns the Rust frontend, evaluator, ABI4 C header and generic source-bound
conformance suite. Netdata supplies contexts metadata, paged retained samples, temporary backing and API/output
consumers. Keep that library boundary intact when changing its physical repository layout.

Before changing frontend/evaluator/C ABI/accounting behavior or claiming compatibility, read
[the conformance workflow](../../../.agents/skills/tests-promql-conformance/SKILL.md), the affected contracts and
qualification cohorts. Use the Netdata root's review and approval rules; source moves do not requalify historical binaries.

## Runtime contracts

- Preserve the pinned same-input expression contract. Retained-data approximation belongs to the provider.
- Keep raw samples paged across both metrics and time; exact general quantiles use provider-owned temporary backing.
- Contain ordinary Rust panics at the C boundary. Preserve independent panic=unwind profiles, including release-min.
- Account resource ownership, work, time and concurrency. Implicit caps remain disabled; explicit controls remain active.
- Preserve the pinned AMD64/FMA-on arithmetic, byte/label/regex behavior and supported-target C interface.
- Keep the Cargo workspace, dependency lock and declared MSRV independent of Netdata's other Rust crates.
- Historical receipts retain their actual source/binary identities. IMPORT.json records the earlier extraction.

## Commands

Run these from this directory:

- Build: cargo build --locked --release.
- Fixture units: cargo test --locked --features fixtures --release; create tests/promql-conformance/.tmp first.
- Size-optimized units: cargo test --locked --features fixtures --profile release-min.
- C/C++ embedding: python tests/promql-conformance/c_embedding.py --help.
- Differential qualification: follow tests/promql-conformance/README.md and its pinned original-source inputs.

Use installed toolchains. Do not install another toolchain implicitly or change production services for validation.
