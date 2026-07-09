//! # query_lang
//!
//! An incremental computation engine: a database that caches the results of
//! derived queries, tracks what each result was computed from, and recomputes
//! only what a change actually affects.
//!
//! A compiler runs the same computation over and over as source is edited —
//! parse a file, resolve its names, type-check its definitions, lower it to IR.
//! Most edits touch a fraction of that work, yet a naive compiler redoes all of
//! it on every keystroke. The query model, from Rust's own `salsa` and
//! rust-analyzer, turns each step into a *query*: a pure function whose result is
//! memoized and whose dependencies are recorded as it runs. When an input
//! changes, the engine invalidates only the queries that transitively read it,
//! and reuses everything else. query-lang is that engine, and nothing more — it
//! owns no compiler, no IR, no syntax; it is generic over the queries a consumer
//! defines.
//!
//! ## The model
//!
//! Three pieces:
//!
//! - A [`System`] is the definition of your queries: a [`Key`](System::Key) type
//!   that names a query, a [`Value`](System::Value) type it produces, and a
//!   [`compute`](System::compute) function that derives one from the other.
//! - A [`Database`] holds the [`System`], stores your **inputs**, and caches the
//!   **derived** results. It is the engine: you set inputs and get results, and
//!   it handles caching, dependency tracking, and invalidation.
//! - A [`get`](Database::get) resolves a query. Called from application code it
//!   returns a result; called from inside a `compute` it reads a dependency, and
//!   the engine records that edge automatically.
//!
//! An input is any key whose value you [`set`](Database::set) directly; every
//! other key is derived through `compute`. One `Key` type names both, so a query
//! reads an input and another query the same way.
//!
//! ## Example
//!
//! An input string, a query that parses it to a number, and a query that squares
//! that number. Editing the input recomputes the chain; an edit that leaves the
//! parsed number unchanged stops at the parse via early cutoff.
//!
//! ```
//! use query_lang::{Database, System, QueryError};
//!
//! #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
//! enum Key {
//!     Source,   // an input: the raw text
//!     Parsed,   // = Source parsed as i64
//!     Squared,  // = Parsed * Parsed
//! }
//!
//! struct Pipeline;
//! impl System for Pipeline {
//!     type Key = Key;
//!     type Value = i64;
//!     fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
//!         match key {
//!             // `Source` is an input; this default only applies if it was unset.
//!             Key::Source => Ok(0),
//!             Key::Parsed => Ok(db.get(&Key::Source)?),
//!             Key::Squared => {
//!                 let n = db.get(&Key::Parsed)?;
//!                 Ok(n * n)
//!             }
//!         }
//!     }
//! }
//!
//! let mut db = Database::new(Pipeline);
//! db.set(Key::Source, 12);
//! assert_eq!(db.get(&Key::Squared)?, 144);
//! assert_eq!(db.stats().computed, 2); // Source is a set input; Parsed and Squared ran
//!
//! // Ask again with no edit: a hit, nothing recomputes.
//! assert_eq!(db.get(&Key::Squared)?, 144);
//! assert_eq!(db.stats().hits, 1);
//! # Ok::<(), QueryError>(())
//! ```
//!
//! ## Features
//!
//! - `std` (default) — the standard library. Without it the crate is
//!   `#![no_std]` and needs only `alloc`; the engine itself uses no operating-
//!   system facilities either way.
//! - `serde` — derives [`serde::Serialize`] for [`Revision`] and [`Stats`] so a
//!   database's version and cache metrics can be logged or inspected.
//!
//! ## Stability
//!
//! Pre-1.0 and in active development. The public surface is being designed across
//! the 0.x series and frozen at `1.0.0`. See
//! [`docs/API.md`](https://github.com/jamesgober/query-lang/blob/main/docs/API.md)
//! and [`dev/ROADMAP.md`](https://github.com/jamesgober/query-lang/blob/main/dev/ROADMAP.md).

#![cfg_attr(not(feature = "std"), no_std)]
#![cfg_attr(docsrs, feature(doc_cfg))]
#![deny(missing_docs)]
#![forbid(unsafe_code)]
#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::todo,
    clippy::unimplemented,
    clippy::unreachable,
    clippy::dbg_macro,
    clippy::print_stdout,
    clippy::print_stderr
)]

extern crate alloc;

mod database;
mod error;
mod revision;
mod stats;
mod system;

pub use database::Database;
pub use error::QueryError;
pub use revision::Revision;
pub use stats::Stats;
pub use system::System;

/// Compiles and runs the `rust` code blocks in `README.md` and `docs/API.md` as
/// part of `cargo test`, so the published examples cannot drift from the API.
///
/// Present only while collecting doctests (`#[cfg(doctest)]`); it is not part of
/// the public surface and does not appear in the built library or its docs.
#[cfg(doctest)]
#[doc = include_str!("../README.md")]
#[doc = include_str!("../docs/API.md")]
pub struct MarkdownDocTests;
