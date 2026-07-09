//! The database version counter that drives incremental validation.

use core::fmt;

/// A monotonic version stamp for the database.
///
/// Every time an input changes, the [`Database`](crate::Database) advances its
/// revision by one. The engine never compares values to decide whether a cached
/// query is still good — it compares revisions, which is a single integer compare
/// regardless of how large the cached value is. Two stamps live on every memoized
/// query: the revision it was last *verified* at, and the revision its value last
/// *changed* at. A query is still valid when none of its dependencies changed
/// after it was last verified, and that whole judgement reduces to `>` on
/// `Revision`.
///
/// Revisions are opaque and ordered: newer revisions compare greater than older
/// ones. The concrete number is exposed through [`as_u64`](Self::as_u64) for
/// logging and tests, but carries no meaning beyond its order.
///
/// # Examples
///
/// The revision advances only when an input actually changes value:
///
/// ```
/// use query_lang::{Database, System, QueryError};
///
/// struct S;
/// impl System for S {
///     type Key = u32;
///     type Value = u32;
///     fn compute(&self, _db: &Database<Self>, key: &u32) -> Result<u32, QueryError> {
///         Ok(*key)
///     }
/// }
///
/// let mut db = Database::new(S);
/// let start = db.revision();
///
/// db.set(1, 10);
/// assert!(db.revision() > start);          // a new input advanced the clock
///
/// let after_first = db.revision();
/// db.set(1, 10);                            // same value — no real change
/// assert_eq!(db.revision(), after_first);   // the clock did not move
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Revision(u64);

impl Revision {
    /// The revision of a freshly created database, before any input is set.
    pub(crate) const START: Revision = Revision(0);

    /// The next revision after this one.
    ///
    /// Saturating rather than wrapping: after `u64::MAX` mutations the counter
    /// stops advancing instead of wrapping back to an older-looking value, which
    /// would silently mark every cached query as current. Reaching that bound
    /// requires more than eighteen quintillion input mutations in one process, so
    /// in practice the saturation never fires — it exists so the arithmetic can
    /// never overflow or panic.
    pub(crate) fn next(self) -> Revision {
        Revision(self.0.saturating_add(1))
    }

    /// The underlying counter value.
    ///
    /// Useful for logging and assertions. The number has no meaning on its own;
    /// only the order between two revisions is significant.
    ///
    /// # Examples
    ///
    /// ```
    /// use query_lang::{Database, System, QueryError};
    ///
    /// struct S;
    /// impl System for S {
    ///     type Key = u32;
    ///     type Value = u32;
    ///     fn compute(&self, _db: &Database<Self>, k: &u32) -> Result<u32, QueryError> { Ok(*k) }
    /// }
    ///
    /// let mut db = Database::new(S);
    /// assert_eq!(db.revision().as_u64(), 0);
    /// db.set(1, 1);
    /// assert_eq!(db.revision().as_u64(), 1);
    /// ```
    #[must_use]
    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }
}

impl fmt::Display for Revision {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_start_is_zero() {
        assert_eq!(Revision::START.as_u64(), 0);
    }

    #[test]
    fn test_next_advances_by_one() {
        assert_eq!(Revision::START.next().as_u64(), 1);
        assert_eq!(Revision::START.next().next().as_u64(), 2);
    }

    #[test]
    fn test_ordering_reflects_age() {
        assert!(Revision::START.next() > Revision::START);
        assert!(Revision::START < Revision::START.next());
    }

    #[test]
    fn test_next_saturates_at_max() {
        let max = Revision(u64::MAX);
        assert_eq!(max.next(), max);
    }

    #[test]
    fn test_display_is_prefixed() {
        use alloc::string::ToString;
        assert_eq!(Revision(7).to_string(), "r7");
    }
}
