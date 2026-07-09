//! Cumulative counters describing how the engine spent its work.

use core::fmt;

/// A snapshot of how a [`Database`](crate::Database) resolved its queries.
///
/// Incremental compilation is only worth its complexity if it actually avoids
/// work, and the only way to know it does is to count. Every derived query
/// resolution takes exactly one of three paths, and `Stats` counts each:
///
/// - **`computed`** — the query ran its [`compute`](crate::System::compute)
///   function. This is the expensive path: a cache miss, or a dependency that
///   genuinely changed and forced a recomputation.
/// - **`validated`** — the query was stale (something changed since it was last
///   checked) but re-examining its dependencies proved none of them actually
///   changed its inputs, so the cached value was reused without recomputing.
///   This is *early cutoff*, the property that makes the engine fast: a change
///   that does not alter a query's inputs does not recompute it.
/// - **`hits`** — the query was already verified at the current revision and
///   returned immediately, without even checking dependencies.
///
/// The counters are cumulative over the life of the database and only ever
/// increase. Snapshot them with [`Database::stats`](crate::Database::stats)
/// before and after an operation to measure exactly what that operation cost.
///
/// # Examples
///
/// After a run, most re-queries of an unchanged graph are hits, and a targeted
/// input change recomputes only what depends on it:
///
/// ```
/// use query_lang::{Database, System, QueryError};
///
/// struct Doubler;
/// impl System for Doubler {
///     type Key = u32;
///     type Value = u32;
///     fn compute(&self, db: &Database<Self>, key: &u32) -> Result<u32, QueryError> {
///         Ok(db.get(&(key + 100))? * 2) // reads input (key + 100), doubles it
///     }
/// }
///
/// let mut db = Database::new(Doubler);
/// db.set(101, 5);
///
/// assert_eq!(db.get(&1)?, 10);
/// assert_eq!(db.stats().computed, 1); // first resolution computes
///
/// assert_eq!(db.get(&1)?, 10);
/// assert_eq!(db.stats().hits, 1);     // second is a free hit
/// # Ok::<(), QueryError>(())
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Stats {
    /// The number of times a query ran its `compute` function (a real cache
    /// miss or a forced recomputation).
    pub computed: u64,
    /// The number of times a stale query was revalidated and its cached value
    /// reused because no dependency actually changed its inputs (early cutoff).
    pub validated: u64,
    /// The number of times a query was already current and returned immediately.
    pub hits: u64,
}

impl Stats {
    /// The total number of derived-query resolutions across all three paths.
    ///
    /// This counts work the engine was *asked* to do; the ratio of
    /// [`computed`](Self::computed) to this total is how much of that work
    /// actually cost a recomputation.
    ///
    /// # Examples
    ///
    /// ```
    /// use query_lang::Stats;
    ///
    /// let s = Stats { computed: 2, validated: 3, hits: 5 };
    /// assert_eq!(s.total(), 10);
    /// ```
    #[must_use]
    #[inline]
    pub const fn total(self) -> u64 {
        self.computed
            .saturating_add(self.validated)
            .saturating_add(self.hits)
    }
}

impl fmt::Display for Stats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "computed={}, validated={}, hits={}",
            self.computed, self.validated, self.hits
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::string::ToString;

    #[test]
    fn test_default_is_all_zero() {
        let s = Stats::default();
        assert_eq!(s.computed, 0);
        assert_eq!(s.validated, 0);
        assert_eq!(s.hits, 0);
        assert_eq!(s.total(), 0);
    }

    #[test]
    fn test_total_sums_all_paths() {
        let s = Stats {
            computed: 1,
            validated: 2,
            hits: 4,
        };
        assert_eq!(s.total(), 7);
    }

    #[test]
    fn test_display_lists_counters() {
        let s = Stats {
            computed: 1,
            validated: 2,
            hits: 3,
        };
        assert_eq!(s.to_string(), "computed=1, validated=2, hits=3");
    }
}
