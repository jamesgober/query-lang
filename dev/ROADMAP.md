# query-lang - Roadmap

> Path from scaffold to a stable 1.0. Hard parts are front-loaded; each phase has hard exit criteria.
> Master plan: ../../_strategy/LANG_COLLECTION.md
>
> **Anti-deferral rule:** no listed hard task moves to a later phase unless this file records the move and the reason.

## v0.1.0 - Scaffold (DONE)
Compiles, CI green, structure correct, no domain logic.
- [x] Manifest, README, CHANGELOG, REPS, dual license, CI, deny, clippy, rustfmt.

## v0.2.0 - Core (THE HARD PART, NOT DEFERRED) (DONE)
A query-based incremental compilation framework (salsa/rust-analyzer model).
Dependencies (none) are wired here, when first used.
Exit criteria:
- [x] Every public item has rustdoc + a runnable example.
- [x] Core invariants property-tested (full DIRECTIVES + API authored at this stage).

Delivered: `System` trait, `Database<S>` engine (revision-based validation with
early cutoff), `Revision`, `Stats`, `QueryError`. No first-party dependency wired
(a generic caching/invalidation engine, like the sibling generic crates); the
reasoning is recorded here under the anti-deferral rule. `no_std` on `alloc`,
`#![forbid(unsafe_code)]`, keys in a `BTreeMap` (`Key: Ord`, no hashing dep).

## v1.0.0 - API freeze
Public surface stable and frozen until 2.0.
- [ ] docs/API.md marked stable; SemVer promise recorded.
- [ ] Full test + benchmark suite green on all three platforms.
