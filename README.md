# intentumdiff-adf-parser

[![CI](https://github.com/buchochelliq-labs/intentumdiff-adf-parser/actions/workflows/ci.yml/badge.svg)](https://github.com/buchochelliq-labs/intentumdiff-adf-parser/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Rust 1.95](https://img.shields.io/badge/rust-1.95-orange.svg)](https://www.rust-lang.org/)

The **adf parser plugin** for IntentumDiff — a Wasm component (WASI p2, Component Model)
implementing the `intentdiff:plugin` parser interface: it emits a deterministic
`SemanticNode` tree for adf sources.

## Configuration fidelity

The parser retains its pipeline/activity/parameter/dataset/dependency nodes and
uses the shared SDK configuration projector for the remaining declared fields.
Defaults, retry policies, dependency conditions, endpoints and nested extension
values retain typed payloads and exact JSON key/value byte spans. Object key
order and JSON string escape spelling are not semantic differences; array order
is preserved. Duplicate decoded keys and unsupported annotation shapes return
an explicit error instead of a partial tree.

Activity and declaration types also retain independent typed values, so different
name/type pairs retain distinct semantic identities even when their legacy
display labels match.

Numbers retain their complete source lexemes, including close decimals and
oversized integers. Numeric notation changes such as `1.0` versus `1e0` remain
conservatively distinguishable; this parser does not prove their equivalence
using rounded floating-point values.

## Build

```bash
cargo build --release --target wasm32-wasip2
```

Toolchain: Rust 1.93.0 (pinned in CI). Tests: `cargo test`.

CI is a thin caller of the reusable `parser-ci.yml` owned by
[intentumdiff-plugin-sdk](https://github.com/buchochelliq-labs/intentumdiff-plugin-sdk) —
parser-CI fixes happen once there, not per parser repo. The `.claude/skills/` copies are
stamped from the SDK master; edits belong there.

## Provenance

Migrated files-only (no history) from the IntentumDiff monorepo
(`buchochelliq-labs/intentumdiff`), which remains the archive of record.

License: MIT.
