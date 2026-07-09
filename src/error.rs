//! The error a query resolution returns when it cannot complete.

use core::fmt;

/// The error returned from [`Database::get`](crate::Database::get) and
/// [`System::compute`](crate::System::compute) when a query cannot be resolved.
///
/// The engine resolves a derived query by running its
/// [`compute`](crate::System::compute), which in turn reads other queries. That
/// recursion terminates only when the dependency graph is acyclic. If a query
/// asks — directly or through a chain of other queries — for a result that is
/// itself still being computed, there is no value to return and no way to make
/// progress: the queries form a cycle. Rather than recurse without bound or
/// panic, the engine unwinds the whole chain with [`QueryError::Cycle`].
///
/// The type is [`non_exhaustive`](https://doc.rust-lang.org/reference/attributes/type_system.html):
/// resolution has exactly one failure mode today, and new engine-level failure
/// modes can be added later without breaking a `match` that already handles
/// `Cycle`.
///
/// # Examples
///
/// A query that reads itself cannot resolve:
///
/// ```
/// use query_lang::{Database, System, QueryError};
///
/// struct SelfReferential;
/// impl System for SelfReferential {
///     type Key = u32;
///     type Value = u32;
///     fn compute(&self, db: &Database<Self>, key: &u32) -> Result<u32, QueryError> {
///         // Asking for the very key being computed closes a cycle.
///         db.get(key)
///     }
/// }
///
/// let db = Database::new(SelfReferential);
/// assert_eq!(db.get(&1), Err(QueryError::Cycle));
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum QueryError {
    /// A query depended on itself, directly or through a chain of other queries.
    ///
    /// The dependency graph must be acyclic. When it is not, the query that
    /// closed the cycle — and every query waiting on it — resolves to this
    /// error. The fix is in the query definitions: break the cyclic dependency,
    /// usually by splitting the offending query so the two halves no longer wait
    /// on each other.
    Cycle,
}

impl fmt::Display for QueryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            QueryError::Cycle => f.write_str("query cycle detected: a query depends on itself"),
        }
    }
}

impl core::error::Error for QueryError {}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_cycle_display_is_descriptive() {
        assert_eq!(
            QueryError::Cycle.to_string(),
            "query cycle detected: a query depends on itself"
        );
    }

    #[test]
    fn test_error_trait_is_implemented() {
        fn assert_error<E: core::error::Error>(_: &E) {}
        assert_error(&QueryError::Cycle);
    }

    #[test]
    fn test_equality() {
        assert_eq!(QueryError::Cycle, QueryError::Cycle);
    }
}
