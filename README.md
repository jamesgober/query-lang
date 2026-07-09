<h1 align="center">
    <img width="99" alt="Rust logo" src="https://raw.githubusercontent.com/jamesgober/rust-collection/72baabd71f00e14aa9184efcb16fa3deddda3a0a/assets/rust-logo.svg">
    <br>
    <b>query-lang</b>
    <br>
    <sub><sup>QUERY COMPILATION</sup></sub>
</h1>

<div align="center">
    <a href="https://crates.io/crates/query-lang"><img alt="Crates.io" src="https://img.shields.io/crates/v/query-lang"></a>
    <a href="https://crates.io/crates/query-lang"><img alt="Downloads" src="https://img.shields.io/crates/d/query-lang?color=%230099ff"></a>
    <a href="https://docs.rs/query-lang"><img alt="docs.rs" src="https://img.shields.io/docsrs/query-lang"></a>
    <a href="https://github.com/jamesgober/query-lang/actions"><img alt="CI" src="https://github.com/jamesgober/query-lang/actions/workflows/ci.yml/badge.svg"></a>
    <a href="https://github.com/rust-lang/rfcs/blob/master/text/2495-min-rust-version.md"><img alt="MSRV" src="https://img.shields.io/badge/MSRV-1.85%2B-blue"></a>
</div>

<br>

<div align="left">
    <p>
        <strong>query-lang</strong> is an incremental computation engine: a database that caches the results of derived queries, records what each result was computed from, and recomputes only what a change actually affects. It is the model behind Rust's own <code>salsa</code> and rust-analyzer, distilled to a small, dependency-free core that is generic over the queries you define.
    </p>
    <p>
        A compiler runs the same work over and over as source is edited — parse, resolve names, type-check, lower to IR. Most edits touch a fraction of that work, yet a naive compiler redoes all of it on every keystroke. The query model turns each step into a memoized function whose dependencies are tracked as it runs; when an input changes, the engine invalidates only the queries that transitively read it and reuses everything else. That is what keeps an editor responsive on a large project, and it is all query-lang does — it owns no compiler, no IR, no syntax.
    </p>
    <br>
    <hr>
    <p>
        <strong>MSRV is 1.85+</strong> (Rust 2024 edition). <code>no_std</code>-compatible (needs only <code>alloc</code>), <code>#![forbid(unsafe_code)]</code>, and wires no first-party dependency.
    </p>
    <blockquote>
        <strong>Status: pre-1.0, in active development.</strong> The public API is being designed across the 0.x series and frozen at <code>1.0.0</code>. See <a href="./CHANGELOG.md"><code>CHANGELOG.md</code></a> and <a href="./dev/ROADMAP.md"><code>ROADMAP</code></a>.
    </blockquote>
</div>

<hr>
<br>

## The model

Three pieces, and the whole surface fits in your head:

- A **[`System`](./docs/API.md#system)** is the definition of your queries: a `Key` type that names a query, a `Value` type it produces, and a `compute` function that derives one from the other.
- A **[`Database`](./docs/API.md#database)** holds the `System`, stores your **inputs**, and caches the **derived** results. You set inputs and get results; it handles caching, dependency tracking, and invalidation.
- A **[`get`](./docs/API.md#databaseget)** resolves a query. From application code it returns a result; from inside a `compute` it reads a dependency — and the engine records that edge automatically.

An **input** is any key whose value you `set` directly; every other key is **derived** through `compute`. One `Key` type names both, so a query reads an input and another query the same way.

<br>

Every resolution takes one of three paths, and the engine counts each in [`Stats`](./docs/API.md#stats):

| Path | When | Cost |
|---|---|---|
| **Hit** | The query was already verified at the current revision. | A revision compare; the cached value is returned without touching dependencies. |
| **Validated** | The query is stale, but no dependency actually changed its inputs. | The dependencies are re-examined; the cached value is reused. This is *early cutoff*. |
| **Computed** | A genuine miss, or a dependency that truly changed. | `compute` runs and the new value is cached. |

*Early cutoff* is the property that makes the engine worth its complexity: when a recomputed query produces the same value it had before, queries that depend on it are validated rather than recomputed, so a local edit does not cascade through the whole graph.

<hr>
<br>

## Installation

```toml
[dependencies]
query-lang = "0.2"
```

Or from the terminal:

```bash
cargo add query-lang
```

MSRV: Rust 1.85 (Rust 2024 edition).

<hr>
<br>

## Quick start

An input, a query that parses it, and a query that squares the result. Editing the input recomputes the chain; asking again with no edit is a free hit.

```rust
use query_lang::{Database, System, QueryError};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Source,   // an input: the raw value
    Parsed,   // = Source
    Squared,  // = Parsed * Parsed
}

struct Pipeline;
impl System for Pipeline {
    type Key = Key;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
        match key {
            Key::Source => Ok(0), // default if the input was never set
            Key::Parsed => Ok(db.get(&Key::Source)?),
            Key::Squared => {
                let n = db.get(&Key::Parsed)?;
                Ok(n * n)
            }
        }
    }
}

let mut db = Database::new(Pipeline);
db.set(Key::Source, 12);
assert_eq!(db.get(&Key::Squared)?, 144);
assert_eq!(db.stats().computed, 2); // Source is a set input; Parsed and Squared ran

// Ask again with no edit: a cache hit, nothing recomputes.
assert_eq!(db.get(&Key::Squared)?, 144);
assert_eq!(db.stats().hits, 1);
# Ok::<(), QueryError>(())
```

<br>

### Values that are expensive to clone

The engine clones a value to hand it back and compares old against new for early cutoff. Wrap a large result in an `Arc` so a clone bumps a refcount rather than copying, and the comparison stays cheap:

```rust
use std::sync::Arc;
use query_lang::{Database, System, QueryError};

struct Ast;
impl System for Ast {
    type Key = u32;
    type Value = Arc<Vec<String>>; // a big parsed tree, shared not copied
    fn compute(&self, db: &Database<Self>, file: &u32) -> Result<Arc<Vec<String>>, QueryError> {
        // ... parse file `file` into tokens ...
        Ok(Arc::new(vec![format!("tokens-of-{file}")]))
    }
}

let db = Database::new(Ast);
let a = db.get(&1)?;
let b = db.get(&1)?;      // second call is a hit
assert!(Arc::ptr_eq(&a, &b)); // and hands back the very same allocation
# Ok::<(), QueryError>(())
```

<hr>
<br>

## Examples

Two runnable examples ship in [`examples/`](./examples):

- **Spreadsheet** — cells are inputs, formulas are derived queries; editing a cell recomputes only the formulas that transitively read it.
  ```bash
  cargo run --example spreadsheet
  ```
- **Build pipeline** — a miniature compiler front end (`source → tokens → symbol count → report`) that shows early cutoff: reformatting the source reruns only the tokenizer; the rest is reused.
  ```bash
  cargo run --example build_pipeline
  ```

<hr>
<br>

## Performance

The engine's own per-query overhead is a `BTreeMap` lookup and a revision compare. The benchmarks in [`benches/`](./benches) exercise the three resolution paths directly (Windows x86_64 and Linux/WSL2, Rust stable, release profile):

| Benchmark | What it measures | Time |
|---|---|---|
| `chain/cache_hit` | Re-resolving an already-current query (a hit). | ~19–38 ns |
| `chain/cold_build/256` | First resolution of a 256-deep query chain. | ~55 µs |
| `chain/edit_rebuild/256` | Editing the leaf input, rebuilding a 256-deep chain. | ~46 µs |
| `wide/edit_one_of/256` | Editing one input in a 256-wide sum (one branch recomputes, 255 validate). | ~42 µs |

Run them yourself:

```bash
cargo bench --bench bench
```

Criterion writes per-benchmark reports to `target/criterion/`. Numbers vary by CPU; use the trend across runs, not a single absolute.

<hr>
<br>

## Design notes

- **Dependencies are recorded, not declared.** A query's dependency set is whatever it *read* on its last run, so a query that branches on its inputs is tracked exactly — it depends on the branch it actually took, and nothing more.
- **Revisions, not value diffs, drive validation.** Validity is a single integer compare regardless of how large a cached value is; values are compared only once, at the point a query recomputes, to decide early cutoff.
- **Single-threaded by design.** Resolution walks a shared cache and a dependency stack through interior mutability — correct and allocation-light on one thread, with no atomic overhead. Run independent databases on separate threads for parallelism.
- **Cycles are an error, never a panic or a hang.** A query that depends on itself resolves to `QueryError::Cycle`; the resolution chain unwinds cleanly and the database stays usable.
- **`no_std` and dependency-free.** The engine uses only `alloc` and wires no first-party crate — keys live in a `BTreeMap`, so the requirement is `Key: Ord` rather than a hashing dependency.

<hr>
<br>

## Testing

The suite runs on Windows, Linux (WSL2 Ubuntu), and macOS through the CI matrix, on stable and the 1.85 MSRV:

```bash
cargo test                       # unit + integration + doctests
cargo test --all-features        # adds serde coverage
cargo clippy --all-targets --all-features -- -D warnings
cargo bench --bench bench
```

Property tests in [`tests/proptests.rs`](./tests/proptests.rs) hold the engine to its core invariants over a wide space of edit sequences: an incrementally maintained result always equals a from-scratch computation, the revision is monotonic, and an unchanged input never recomputes. Every `rust` example in this README and in [`docs/API.md`](./docs/API.md) is compiled and run as a doctest, so the published examples cannot drift from the API.

<hr>
<br>

## Cross-platform support

- Linux (x86_64, aarch64)
- macOS (x86_64, Apple Silicon)
- Windows (x86_64)

The engine uses no operating-system facilities and no platform-specific code; behaviour is identical on every target.

<hr>
<br>

## Contributing

See [`dev/DIRECTIVES.md`](./dev/DIRECTIVES.md) for engineering standards and the definition of done. Before a PR: `cargo fmt --all`, `cargo clippy --all-targets --all-features -- -D warnings`, and `cargo test --all-features` must be clean.

<br>

<div id="license">
    <h2>License</h2>
    <p>Licensed under either of</p>
    <ul>
        <li><b>Apache License, Version 2.0</b> &mdash; <a href="./LICENSE-APACHE">LICENSE-APACHE</a></li>
        <li><b>MIT License</b> &mdash; <a href="./LICENSE-MIT">LICENSE-MIT</a></li>
    </ul>
    <p>at your option.</p>
</div>

<div align="center">
  <h2></h2>
  <sup>COPYRIGHT <small>&copy;</small> 2026 <strong>James Gober <me@jamesgober.com>.</strong></sup>
</div>
