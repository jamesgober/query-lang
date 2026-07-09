//! The trait a consumer implements to define its derived queries.

use crate::database::Database;
use crate::error::QueryError;

/// The definition of a query system: how every derived query is computed.
///
/// A consumer implements `System` once to describe an entire incremental
/// computation. It ties together three things: the [`Key`](Self::Key) that names
/// a query, the [`Value`](Self::Value) a query produces, and the
/// [`compute`](Self::compute) function that turns one into the other. The
/// [`Database`](crate::Database) supplies everything else — caching, dependency
/// tracking, and invalidation — and calls `compute` only when it must.
///
/// # Keys, inputs, and derived queries
///
/// A single `Key` type names every query in the system, usually an `enum` with
/// one variant per kind of query (`Key::Source(FileId)`, `Key::Ast(FileId)`,
/// `Key::TypeOf(DefId)`, …). A key is an **input** once its value is placed into
/// the database with [`Database::set`](crate::Database::set); every other key is
/// **derived**, and its value comes from `compute`. The same key type covers
/// both, so a query reads an input and another derived query the same way —
/// through [`Database::get`](crate::Database::get) — and the engine records the
/// dependency either way.
///
/// # The contract on `compute`
///
/// `compute` must be a pure function of the queries it reads. It may read inputs
/// and other derived queries through the `db` handle it is given, and it must
/// read *every* value it depends on through that handle — a value pulled in from
/// outside (a global, the clock, the filesystem read directly) is invisible to
/// the engine and will not trigger invalidation when it changes, leaving the
/// cache serving stale results. Given the same inputs, `compute` must return the
/// same value; the engine relies on that to reuse cached results safely.
///
/// # Requirements on the associated types
///
/// - `Key: Clone + Ord` — keys are stored in the dependency graph and the memo
///   table (a `BTreeMap`, so `Ord` rather than `Hash`; this keeps the engine
///   `no_std`- and dependency-free). Cloning a key should be cheap; prefer small
///   copyable keys or interned identifiers over owned strings.
/// - `Value: Clone + Eq` — the engine clones a value to hand it back and compares
///   the new value against the old one to decide whether a recomputation actually
///   changed anything. That comparison is *early cutoff*: when a recomputed value
///   equals its predecessor, queries that depend on it are not recomputed. Make
///   values cheap to clone and compare — wrap large results in an
///   [`Arc`](std::sync::Arc) so a clone bumps a refcount rather than copying.
///
/// # Examples
///
/// A two-layer system: an input number, and a derived query that squares it.
///
/// ```
/// use query_lang::{Database, System, QueryError};
///
/// // One enum names every query. `Base` values are set as inputs; `Squared`
/// // values are computed from a `Base`.
/// #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
/// enum Key {
///     Base,
///     Squared,
/// }
///
/// struct Arithmetic;
/// impl System for Arithmetic {
///     type Key = Key;
///     type Value = i64;
///     fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
///         match key {
///             Key::Base => Ok(0), // a default if `Base` was never set as an input
///             Key::Squared => {
///                 let base = db.get(&Key::Base)?;
///                 Ok(base * base)
///             }
///         }
///     }
/// }
///
/// let mut db = Database::new(Arithmetic);
/// db.set(Key::Base, 9);
/// assert_eq!(db.get(&Key::Squared)?, 81);
/// # Ok::<(), QueryError>(())
/// ```
pub trait System: Sized {
    /// The identifier that names a query. Usually an `enum` with one variant per
    /// kind of query. Must be cheap to clone and totally ordered.
    type Key: Clone + Ord;

    /// The value a query produces. Cloned to return and compared for early
    /// cutoff, so it should be cheap to clone and compare (wrap large payloads in
    /// an [`Arc`](std::sync::Arc)).
    type Value: Clone + Eq;

    /// Compute the value of a derived query.
    ///
    /// The engine calls this only on a cache miss or when a dependency has
    /// genuinely changed — never for a key that is currently a set input, and
    /// never when a cached value is still valid. Read every dependency through
    /// `db` so the engine can track it; see the [trait
    /// contract](Self#the-contract-on-compute).
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Cycle`] if resolving a dependency closes a cycle
    /// back onto a query still being computed. Propagate it with `?`; do not
    /// attempt to recover from it inside `compute`, as the whole resolution
    /// chain is already unwinding.
    fn compute(&self, db: &Database<Self>, key: &Self::Key) -> Result<Self::Value, QueryError>;
}
