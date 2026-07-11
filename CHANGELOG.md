<h1 align="center">
    <img width="90px" height="auto" src="https://raw.githubusercontent.com/jamesgober/jamesgober/main/media/icons/hexagon-3.svg" alt="Triple Hexagon">
    <br><b>CHANGELOG</b>
</h1>
<p>
  All notable changes to <code>query-lang</code> will be documented in this file. The format is based on <a href="https://keepachangelog.com/en/1.1.0/">Keep a Changelog</a>,
  and this project adheres to <a href="https://semver.org/spec/v2.0.0.html/">Semantic Versioning</a>.
</p>

---

## [Unreleased]

### Added

### Changed

### Fixed

### Security

---

## [1.0.0] - 2026-07-11

API freeze. The public surface introduced in 0.2.0 is now stable and frozen under
Semantic Versioning: no breaking changes ship before `2.0`. There are no code
changes from 0.2.0 — this release marks the contract as fixed and completes the
roadmap.

### Changed

- Bumped the crate version to `1.0.0` and declared the public API stable.
- `docs/API.md` marked stable with a recorded SemVer promise: the surface, the
  resolution semantics (unchanged-input no-op, early cutoff), the
  `#[non_exhaustive]` error, the `serde` representations, and MSRV 1.85 as a
  compatibility surface.

---

## [0.2.0] - 2026-07-08

The core, and the hard part of the roadmap: the scaffold becomes a working
incremental computation engine — a query database with automatic dependency
tracking, revision-based invalidation, and early cutoff. The public surface is
deliberately small (one trait, one database, three supporting types) and generic
over the queries a consumer defines, so it binds to no concrete compiler and
wires no first-party dependency.

### Added

- `System` — the trait a consumer implements to define its queries: a `Key` type,
  a `Value` type, and a `compute` function. Reading dependencies through the
  database handle is what the engine records as the dependency graph.
- `Database<S>` — the engine. `new` builds it, `set` seeds inputs (advancing the
  revision only on a real change), and `get` resolves a query, computing and
  caching it as needed with automatic invalidation and early cutoff. Also exposes
  `revision`, `stats`, and `system`.
- `Revision` — the monotonic version clock that drives validation by integer
  compare rather than value diffing.
- `Stats` — cumulative counters (`computed`, `validated`, `hits`) for measuring
  exactly what an operation cost, plus a `total`.
- `QueryError` — the resolution error, with a `Cycle` variant for a query that
  depends on itself; the graph unwinds cleanly instead of panicking or hanging.
- Two runnable examples (`spreadsheet`, `build_pipeline`), Criterion benchmarks
  for the hit / cold-build / recompute paths, property tests holding the engine
  to its core invariants, and full `docs/API.md`.

### Changed

- Bumped the crate version to `0.2.0`.

### Fixed

- Corrected invalid TOML in `Cargo.toml` (`keywords` and `categories` were
  unquoted) that prevented the manifest from parsing.
- Aligned the `clippy.toml` MSRV (`1.87` → `1.85`) with `Cargo.toml`.

---

## [0.1.0] - 2026-06-18

Initial scaffold and repository bootstrap. No domain logic yet &mdash; this release establishes the structure, tooling, and quality gates the implementation will be built on.

### Added

- `Cargo.toml` with crate metadata, Rust 2024 edition, MSRV 1.85.
- Dual `Apache-2.0 OR MIT` license files.
- `README.md`, `CHANGELOG.md`, and a documentation skeleton.
- `REPS.md` compliance baseline.
- `.github/workflows/ci.yml` CI matrix; `deny.toml`, `clippy.toml`, `rustfmt.toml`.
- `dev/DIRECTIVES.md` and `dev/ROADMAP.md` (committed engineering standards + plan).

[Unreleased]: https://github.com/jamesgober/query-lang/compare/v1.0.0...HEAD
[1.0.0]: https://github.com/jamesgober/query-lang/compare/v0.2.0...v1.0.0
[0.2.0]: https://github.com/jamesgober/query-lang/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/jamesgober/query-lang/releases/tag/v0.1.0
