# query-lang &mdash; API Reference

> Complete reference for every public item in `query-lang`, with examples.
> **Status: pre-1.0.** The surface below is being designed across the 0.x series
> and frozen at `1.0.0`; until then a minor release may refine it. See
> [`../dev/ROADMAP.md`](../dev/ROADMAP.md).

<sub>Copyright &copy; 2026 <strong>James Gober</strong>.</sub>

## Table of contents

- [Overview](#overview)
- [Installation](#installation)
- [Quick start](#quick-start)
- [The model](#the-model)
- [`System`](#system)
  - [`System::Key`](#systemkey)
  - [`System::Value`](#systemvalue)
  - [`System::compute`](#systemcompute)
- [`Database`](#database)
  - [`Database::new`](#databasenew)
  - [`Database::set`](#databaseset)
  - [`Database::get`](#databaseget)
  - [`Database::revision`](#databaserevision)
  - [`Database::stats`](#databasestats)
  - [`Database::system`](#databasesystem)
- [`Revision`](#revision)
- [`Stats`](#stats)
- [`QueryError`](#queryerror)
- [Feature flags](#feature-flags)
- [Versioning](#versioning)

---

## Overview

query-lang is an incremental computation engine — the model behind `salsa` and
rust-analyzer, reduced to a small, dependency-free core. You describe a set of
queries once; the engine stores the base facts (**inputs**), caches the computed
results (**derived queries**), records what each result was read from, and
recomputes only what a change actually affects.

Four public types and one trait make up the whole surface:

| Item | Role |
|---|---|
| [`System`](#system) | The trait you implement to define your queries. |
| [`Database`](#database) | The engine: stores inputs, caches results, tracks dependencies. |
| [`Revision`](#revision) | The version clock that drives validation. |
| [`Stats`](#stats) | Cumulative counters for how the engine spent its work. |
| [`QueryError`](#queryerror) | The error a resolution returns (a query cycle). |

The crate is `#![forbid(unsafe_code)]`, `no_std`-compatible (needs only `alloc`),
and wires no first-party dependency.

---

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

---

## Quick start

Define a system, set an input, and get a derived result. Ask again with no edit
and the result is a cache hit.

```rust
use query_lang::{Database, System, QueryError};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Radius,        // an input
    Circumference, // = 2 * PI * Radius, in whole units
}

struct Circle;
impl System for Circle {
    type Key = Key;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
        match key {
            Key::Radius => Ok(0), // default if never set
            Key::Circumference => Ok(db.get(&Key::Radius)? * 628 / 100),
        }
    }
}

let mut db = Database::new(Circle);
db.set(Key::Radius, 10);
assert_eq!(db.get(&Key::Circumference)?, 62);

// No edit: the second query is a hit.
let before = db.stats().hits;
assert_eq!(db.get(&Key::Circumference)?, 62);
assert_eq!(db.stats().hits, before + 1);
# Ok::<(), QueryError>(())
```

---

## The model

A single `Key` type names every query in a system. A key is an **input** once its
value is placed into the database with [`Database::set`](#databaseset); every
other key is **derived**, and its value comes from [`compute`](#systemcompute). A
query reads an input and another derived query the same way — through
[`Database::get`](#databaseget) — and the engine records the dependency either
way.

Every resolution of a derived query takes one of three paths, counted in
[`Stats`](#stats):

- **Computed** — `compute` ran (a cache miss, or a dependency that truly changed).
- **Validated** — the query was stale, but re-examining its dependencies showed
  none had changed its inputs, so the cached value was reused (*early cutoff*).
- **Hit** — the query was already current and returned immediately.

The engine never compares whole values to decide validity; it compares
[`Revision`](#revision) stamps, which is one integer compare regardless of value
size. Values are compared only at the moment a query recomputes, to decide
whether the new value differs from the old — the check that drives early cutoff.

---

## `System`

```rust,ignore
pub trait System: Sized {
    type Key: Clone + Ord;
    type Value: Clone + Eq;
    fn compute(&self, db: &Database<Self>, key: &Self::Key) -> Result<Self::Value, QueryError>;
}
```

The definition of a query system: how every derived query is computed. You
implement `System` once to describe an entire incremental computation. It ties
together the [`Key`](#systemkey) that names a query, the [`Value`](#systemvalue) a
query produces, and the [`compute`](#systemcompute) function that turns one into
the other. The [`Database`](#database) supplies everything else — caching,
dependency tracking, and invalidation.

The `System` value itself can hold immutable configuration a `compute` reads (a
grammar, a set of options); it is borrowed, not mutated, during resolution.

### `System::Key`

```rust,ignore
type Key: Clone + Ord;
```

The identifier that names a query, usually an `enum` with one variant per kind of
query (`Key::Source(FileId)`, `Key::Ast(FileId)`, `Key::TypeOf(DefId)`, …). Keys
are stored in the dependency graph and the memo table (a `BTreeMap`, hence `Ord`
rather than `Hash` — this keeps the engine `no_std`- and dependency-free).

Cloning a key should be cheap: prefer small copyable keys or interned identifiers
over owned strings.

```rust
use query_lang::{Database, System, QueryError};

// A key that names two kinds of query over a file id.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    Source(u32),
    LineCount(u32),
}

struct Files;
impl System for Files {
    type Key = Key;
    type Value = usize;
    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<usize, QueryError> {
        match key {
            Key::Source(_) => Ok(0),
            Key::LineCount(f) => Ok(db.get(&Key::Source(*f))?),
        }
    }
}

let mut db = Database::new(Files);
db.set(Key::Source(1), 42);
assert_eq!(db.get(&Key::LineCount(1))?, 42);
# Ok::<(), QueryError>(())
```

### `System::Value`

```rust,ignore
type Value: Clone + Eq;
```

The value a query produces. The engine clones a value to hand it back and
compares the new value against the old to decide whether a recomputation changed
anything — so it should be cheap to clone and compare. Wrap a large result in an
[`Arc`](https://doc.rust-lang.org/std/sync/struct.Arc.html) so a clone bumps a
refcount rather than copying, and equality short-circuits on pointer identity in
the common case.

```rust
use std::sync::Arc;
use query_lang::{Database, System, QueryError};

struct Parser;
impl System for Parser {
    type Key = u32;
    // A large parsed payload, shared rather than copied.
    type Value = Arc<Vec<u32>>;
    fn compute(&self, _db: &Database<Self>, file: &u32) -> Result<Arc<Vec<u32>>, QueryError> {
        Ok(Arc::new(vec![*file; 3]))
    }
}

let db = Database::new(Parser);
let a = db.get(&7)?;
let b = db.get(&7)?;             // a hit
assert!(Arc::ptr_eq(&a, &b));   // same allocation handed back
# Ok::<(), QueryError>(())
```

### `System::compute`

```rust,ignore
fn compute(&self, db: &Database<Self>, key: &Self::Key) -> Result<Self::Value, QueryError>;
```

Compute the value of a derived query. The engine calls this only on a cache miss
or when a dependency has genuinely changed — never for a key that is currently a
set input, and never when a cached value is still valid.

**Parameters**

- `&self` — the query system, for reading immutable configuration.
- `db` — the database handle. Read every dependency through it (`db.get(&other)`)
  so the engine can track the edge.
- `key` — the query to compute.

**Contract.** `compute` must be a pure function of the queries it reads. It must
read *every* value it depends on through `db` — a value pulled in from outside (a
global, the clock, a direct file read) is invisible to the engine and will not
trigger invalidation when it changes, leaving the cache serving stale results.
Given the same inputs, `compute` must return the same value.

**Errors.** Returns [`QueryError::Cycle`](#queryerror) if resolving a dependency
closes a cycle back onto a query still being computed. Propagate it with `?`; do
not try to recover from it inside `compute`, as the whole resolution chain is
already unwinding.

Reading one dependency:

```rust
use query_lang::{Database, System, QueryError};

struct Doubler;
impl System for Doubler {
    type Key = u32;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &u32) -> Result<i64, QueryError> {
        // key 0 is an input; every other key doubles key 0.
        if *key == 0 {
            Ok(0)
        } else {
            Ok(db.get(&0)? * 2)
        }
    }
}

let mut db = Database::new(Doubler);
db.set(0, 21);
assert_eq!(db.get(&1)?, 42);
# Ok::<(), QueryError>(())
```

Reading several dependencies, and branching on one — only the branch actually
taken becomes a dependency:

```rust
use query_lang::{Database, System, QueryError};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key { Flag, A, B, Chosen }

struct Switch;
impl System for Switch {
    type Key = Key;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
        match key {
            Key::Flag | Key::A | Key::B => Ok(0),
            Key::Chosen => {
                if db.get(&Key::Flag)? != 0 { db.get(&Key::A) } else { db.get(&Key::B) }
            }
        }
    }
}

let mut db = Database::new(Switch);
db.set(Key::Flag, 1);
db.set(Key::A, 100);
db.set(Key::B, 200);
assert_eq!(db.get(&Key::Chosen)?, 100); // took the A branch; B is not a dependency
# Ok::<(), QueryError>(())
```

---

## `Database`

```rust,ignore
pub struct Database<S: System> { /* private */ }
```

The engine: the store of inputs and the cache of derived results, with automatic
dependency tracking and invalidation. Construct one with [`new`](#databasenew),
seed base facts with [`set`](#databaseset), and ask for results with
[`get`](#databaseget). It also exposes its [`revision`](#databaserevision) clock,
its cache [`stats`](#databasestats), and the [`system`](#databasesystem) it holds.

A `Database` is single-threaded by design: it is not `Sync`, since resolution
walks a shared cache and dependency stack through interior mutability. Drive one
database from one thread; run independent databases on separate threads for
parallelism.

### `Database::new`

```rust,ignore
pub fn new(system: S) -> Self
```

Create an empty database for the given query system. It starts at the initial
revision with no inputs and an empty cache.

```rust
use query_lang::{Database, System, QueryError};

struct S;
impl System for S {
    type Key = u32;
    type Value = u32;
    fn compute(&self, _db: &Database<Self>, k: &u32) -> Result<u32, QueryError> { Ok(*k) }
}

let db = Database::new(S);
assert_eq!(db.revision().as_u64(), 0);
assert_eq!(db.stats().total(), 0);
```

### `Database::set`

```rust,ignore
pub fn set(&mut self, key: S::Key, value: S::Value)
```

Set an input to a value. This is the only way a value enters the database from
outside. Once set, a key is an input: [`get`](#databaseget) returns it directly
and [`compute`](#systemcompute) is never called for it.

**Parameters**

- `key` — the input to set.
- `value` — its new value.

**Behaviour**

- Setting the **same** value a key already holds is a no-op — the revision does
  not advance and nothing that depends on the input is invalidated, so
  re-feeding unchanged facts costs nothing downstream.
- Setting a **different** value advances the [`revision`](#databaserevision),
  which is what later marks dependent queries stale.
- Setting a key that previously held a derived value promotes it to an input and
  discards the stale cached result.

Taking `&mut self` is deliberate: mutating an input is the one operation that can
invalidate cached results, so it is kept distinct from the shared `&self` reads
of [`get`](#databaseget).

```rust
use query_lang::{Database, System, QueryError};

struct S;
impl System for S {
    type Key = u32;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, k: &u32) -> Result<i64, QueryError> {
        if *k == 0 { Ok(0) } else { Ok(db.get(&0)? + 1) }
    }
}

let mut db = Database::new(S);
db.set(0, 41);
let r0 = db.revision();

db.set(0, 41);               // same value
assert_eq!(db.revision(), r0);  // clock did not move

db.set(0, 99);               // new value
assert!(db.revision() > r0);    // clock advanced
assert_eq!(db.get(&1)?, 100);
# Ok::<(), QueryError>(())
```

### `Database::get`

```rust,ignore
pub fn get(&self, key: &S::Key) -> Result<S::Value, QueryError>
```

Resolve a query to its value, computing and caching it as needed. If `key` is a
set input, its value is returned directly. Otherwise the query is derived: a
valid cached value is reused (a hit or an early-cutoff validation), and only a
real miss or a genuinely changed dependency runs [`compute`](#systemcompute).

Call `get` both from application code and, from inside a `compute`, to read the
queries a result depends on — reads through `get` are exactly what the engine
records as dependencies.

**Parameters**

- `key` — the query to resolve.

**Errors.** Returns [`QueryError::Cycle`](#queryerror) if resolving `key`
requires a value still being computed further up the call chain — that is, if the
query graph has a cycle.

Memoized recursion — each subproblem is computed once and cached:

```rust
use query_lang::{Database, System, QueryError};

struct Fib;
impl System for Fib {
    type Key = u64;
    type Value = u64;
    fn compute(&self, db: &Database<Self>, n: &u64) -> Result<u64, QueryError> {
        if *n < 2 { return Ok(*n); }
        Ok(db.get(&(n - 1))?.wrapping_add(db.get(&(n - 2))?))
    }
}

let db = Database::new(Fib);
assert_eq!(db.get(&50)?, 12586269025);
# Ok::<(), QueryError>(())
```

Reusing the cache — an unrelated edit recomputes nothing:

```rust
use query_lang::{Database, System, QueryError};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key { A, B, FromA }

struct S;
impl System for S {
    type Key = Key;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
        match key {
            Key::A | Key::B => Ok(0),
            Key::FromA => Ok(db.get(&Key::A)? + 1), // depends on A, never B
        }
    }
}

let mut db = Database::new(S);
db.set(Key::A, 10);
db.set(Key::B, 20);
assert_eq!(db.get(&Key::FromA)?, 11);
let computed = db.stats().computed;

db.set(Key::B, 999);            // edit an input FromA never read
assert_eq!(db.get(&Key::FromA)?, 11);
assert_eq!(db.stats().computed, computed); // nothing recomputed
# Ok::<(), QueryError>(())
```

### `Database::revision`

```rust,ignore
pub const fn revision(&self) -> Revision
```

The current [`Revision`](#revision) of the database. Advances by one each time
[`set`](#databaseset) gives an input a new value. Useful for asserting that an
operation did (or did not) change any input, and for correlating cache behaviour
with edits in logs.

```rust
use query_lang::{Database, System, QueryError};

struct S;
impl System for S {
    type Key = u32;
    type Value = u32;
    fn compute(&self, _db: &Database<Self>, k: &u32) -> Result<u32, QueryError> { Ok(*k) }
}

let mut db = Database::new(S);
assert_eq!(db.revision().as_u64(), 0);
db.set(1, 10);
db.set(2, 20);
assert_eq!(db.revision().as_u64(), 2);
```

### `Database::stats`

```rust,ignore
pub fn stats(&self) -> Stats
```

A snapshot of the cumulative resolution counters (see [`Stats`](#stats)).
Snapshot before and after an operation and subtract to measure exactly what that
operation cost.

```rust
use query_lang::{Database, System, QueryError};

struct S;
impl System for S {
    type Key = u32;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, k: &u32) -> Result<i64, QueryError> {
        if *k == 0 { Ok(0) } else { Ok(db.get(&0)? + 1) }
    }
}

let mut db = Database::new(S);
db.set(0, 5);

let before = db.stats();
let _ = db.get(&1)?;                 // computes key 1
let after = db.stats();
assert_eq!(after.computed - before.computed, 1);

let _ = db.get(&1)?;                 // now a hit
assert_eq!(db.stats().hits, 1);
# Ok::<(), QueryError>(())
```

### `Database::system`

```rust,ignore
pub const fn system(&self) -> &S
```

A shared reference to the query system backing this database. Handy when the
system holds state a `compute` recorded (a counter, a diagnostics sink) that the
caller wants to read back.

```rust
use std::cell::Cell;
use query_lang::{Database, System, QueryError};

struct Counted { runs: Cell<u32> }
impl System for Counted {
    type Key = u32;
    type Value = u32;
    fn compute(&self, _db: &Database<Self>, k: &u32) -> Result<u32, QueryError> {
        self.runs.set(self.runs.get() + 1);
        Ok(*k)
    }
}

let db = Database::new(Counted { runs: Cell::new(0) });
let _ = db.get(&1)?;
let _ = db.get(&2)?;
assert_eq!(db.system().runs.get(), 2);
# Ok::<(), QueryError>(())
```

---

## `Revision`

```rust,ignore
pub struct Revision(/* private */);
```

A monotonic version stamp for the database. Every time an input changes, the
database advances its revision by one. The engine compares revisions — not
values — to decide whether a cached query is still good, which is a single
integer compare regardless of how large the cached value is.

Revisions are opaque and ordered: newer revisions compare greater than older
ones. The concrete number is exposed through `as_u64` for logging and tests, and
carries no meaning beyond its order. `Revision` implements `Copy`, `Ord`, `Hash`,
`Default`, and `Display` (as `r<n>`).

```rust,ignore
pub const fn as_u64(self) -> u64
```

The underlying counter value.

```rust
use query_lang::{Database, System, QueryError};

struct S;
impl System for S {
    type Key = u32;
    type Value = u32;
    fn compute(&self, _db: &Database<Self>, k: &u32) -> Result<u32, QueryError> { Ok(*k) }
}

let mut db = Database::new(S);
let start = db.revision();
db.set(1, 1);
let next = db.revision();

assert!(next > start);              // ordered
assert_eq!(next.as_u64(), 1);       // and inspectable
assert_eq!(alloc_display(next), "r1");

fn alloc_display(r: query_lang::Revision) -> String { format!("{r}") }
```

---

## `Stats`

```rust,ignore
pub struct Stats {
    pub computed: u64,
    pub validated: u64,
    pub hits: u64,
}
```

A snapshot of how a [`Database`](#database) resolved its queries. The counters are
cumulative over the life of the database and only ever increase.

| Field | Meaning |
|---|---|
| `computed` | Times a query ran its `compute` (a cache miss or a forced recomputation). |
| `validated` | Times a stale query was revalidated and its cached value reused because no dependency changed its inputs (*early cutoff*). |
| `hits` | Times a query was already current and returned immediately. |

`Stats` implements `Copy`, `PartialEq`, `Eq`, `Default`, and `Display`. Behind the
`serde` feature it also derives `Serialize`.

```rust,ignore
pub const fn total(self) -> u64
```

The total number of derived-query resolutions across all three paths. The ratio
of `computed` to `total` is how much of the requested work actually cost a
recomputation.

```rust
use query_lang::Stats;

let s = Stats { computed: 2, validated: 3, hits: 5 };
assert_eq!(s.total(), 10);
assert_eq!(s.to_string(), "computed=2, validated=3, hits=5");
```

Measuring early cutoff end to end — a value-preserving edit validates the
top query instead of recomputing it:

```rust
use query_lang::{Database, System, QueryError};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key { In, Abs, Report }

struct S;
impl System for S {
    type Key = Key;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
        match key {
            Key::In => Ok(0),
            Key::Abs => Ok(db.get(&Key::In)?.abs()),
            Key::Report => Ok(db.get(&Key::Abs)? + 1),
        }
    }
}

let mut db = Database::new(S);
db.set(Key::In, 5);
assert_eq!(db.get(&Key::Report)?, 6);

// -5 has the same absolute value as 5, so `Abs` recomputes to the same result
// and `Report` is validated by early cutoff rather than recomputed.
db.set(Key::In, -5);
assert_eq!(db.get(&Key::Report)?, 6);
assert!(db.stats().validated >= 1);
# Ok::<(), QueryError>(())
```

---

## `QueryError`

```rust,ignore
#[non_exhaustive]
pub enum QueryError {
    Cycle,
}
```

The error returned from [`Database::get`](#databaseget) and
[`System::compute`](#systemcompute) when a query cannot be resolved. Resolution
terminates only when the dependency graph is acyclic; if a query asks — directly
or through a chain — for a result still being computed, there is no value to
return. Rather than recurse without bound or panic, the engine unwinds the whole
chain with `QueryError::Cycle`.

The type is `#[non_exhaustive]`: resolution has one failure mode today, and a
`match` that handles `Cycle` plus a wildcard stays correct if a variant is added
later. `QueryError` implements `Copy`, `Eq`, `Display`, and `core::error::Error`.

```rust
use query_lang::{Database, System, QueryError};

struct SelfReferential;
impl System for SelfReferential {
    type Key = u32;
    type Value = u32;
    fn compute(&self, db: &Database<Self>, key: &u32) -> Result<u32, QueryError> {
        db.get(key) // asks for the very key being computed
    }
}

let db = Database::new(SelfReferential);
assert_eq!(db.get(&1), Err(QueryError::Cycle));

// The database stays usable; a non-cyclic query still resolves.
assert!(db.get(&1).is_err());
```

Handling a cycle gracefully — treat it as a domain sentinel, the way a
spreadsheet shows `#CYCLE!`:

```rust
use query_lang::{Database, System, QueryError};

struct S;
impl System for S {
    type Key = u32;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, k: &u32) -> Result<i64, QueryError> {
        if *k == 0 { db.get(&0) } else { Ok(*k as i64) }
    }
}

let db = Database::new(S);
let shown = match db.get(&0) {
    Ok(v) => v,
    Err(_) => -1, // the only resolution error is a cycle
};
assert_eq!(shown, -1);
assert_eq!(db.get(&5)?, 5);
# Ok::<(), QueryError>(())
```

---

## Feature flags

| Feature | Default | Description |
|---|---|---|
| `std` | yes | Links the standard library. Without it the crate is `#![no_std]` and needs only `alloc`; the engine uses no OS facilities either way. |
| `serde` | no | Derives `serde::Serialize` for [`Revision`](#revision) and [`Stats`](#stats), so a database's version and cache metrics can be logged or inspected. |

```toml
# no_std build:
query-lang = { version = "0.2", default-features = false }

# with serde:
query-lang = { version = "0.2", features = ["serde"] }
```

Feature flags are additive: enabling one never removes or changes existing
behaviour.

---

## Versioning

query-lang follows [Semantic Versioning](https://semver.org/). The crate is
pre-1.0: the surface documented here is being designed across the 0.x series and
a minor release may still refine it. It will be frozen at `1.0.0`, after which no
breaking change ships before `2.0`. See
[`../dev/ROADMAP.md`](../dev/ROADMAP.md) and
[`../CHANGELOG.md`](../CHANGELOG.md).
