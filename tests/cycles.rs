//! Cycles that a query recovers from, panics inside `compute`, and depth limits.
//!
//! A query may catch the `Cycle` error from a dependency and fall back to a
//! default. The memo table then records two queries that depend on each other,
//! and revalidating either one after a later edit must still end in a value or
//! an error, never in unbounded recursion.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::cell::Cell;
use std::panic::{AssertUnwindSafe, catch_unwind};

use query_lang::{Database, QueryError, System};

/// Run `f` on a thread with a small stack, so a runaway recursion fails the
/// test quickly instead of hiding behind a large main-thread stack.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .unwrap();
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum K {
    A,
    B,
    Unrelated,
    Fallback,
}

/// `A` reads `B` but falls back to 0 when `B` fails; `B` reads `A` with `?`.
struct Recovering;

impl System for Recovering {
    type Key = K;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, key: &K) -> Result<i64, QueryError> {
        match key {
            K::A => Ok(db.get(&K::B).unwrap_or(0) + 1),
            K::B => Ok(db.get(&K::A)? * 2),
            K::Unrelated | K::Fallback => Ok(0),
        }
    }
}

#[test]
fn test_recovered_cycle_revalidates_after_an_edit() {
    on_small_stack(|| {
        let mut db = Database::new(Recovering);
        assert_eq!(db.get(&K::A), Ok(1));
        assert_eq!(db.get(&K::B), Ok(2));

        db.set(K::Unrelated, 5);
        // A fresh database gives the same answers, so the revalidated ones must
        // match it.
        assert_eq!(db.get(&K::A), Ok(1));
        assert_eq!(db.get(&K::B), Ok(2));

        db.set(K::Unrelated, 6);
        assert_eq!(db.get(&K::B), Ok(2));
        assert_eq!(db.get(&K::A), Ok(1));
    });
}

#[test]
fn test_recovered_cycle_matches_a_fresh_database() {
    on_small_stack(|| {
        let mut db = Database::new(Recovering);
        for round in 0..20 {
            db.set(K::Unrelated, round);
            let fresh = Database::new(Recovering);
            if round % 2 == 0 {
                assert_eq!(db.get(&K::A), fresh.get(&K::A));
                assert_eq!(db.get(&K::B), fresh.get(&K::B));
            } else {
                assert_eq!(db.get(&K::B), fresh.get(&K::B));
                assert_eq!(db.get(&K::A), fresh.get(&K::A));
            }
        }
    });
}

/// A ring of `n` queries, each falling back to 0 when the next one fails, plus
/// an input at key `n` that every query in the ring reads.
struct Ring(u32);

impl System for Ring {
    type Key = u32;
    type Value = u64;
    fn compute(&self, db: &Database<Self>, key: &u32) -> Result<u64, QueryError> {
        if *key >= self.0 {
            return Ok(0);
        }
        let next = (key + 1) % self.0;
        Ok(db.get(&next).unwrap_or(0) + db.get(&self.0)?)
    }
}

#[test]
fn test_recovered_ring_revalidates_after_an_edit() {
    on_small_stack(|| {
        let mut db = Database::new(Ring(16));
        db.set(16, 1);
        for k in 0..16 {
            assert!(db.get(&k).is_ok());
        }
        for value in 2..10 {
            db.set(16, value);
            for k in (0..16).rev() {
                assert!(db.get(&k).is_ok());
            }
        }
    });
}

/// A query that panics while a switch is on.
struct Panicky {
    explode: Cell<bool>,
}

impl System for Panicky {
    type Key = u32;
    type Value = u32;
    fn compute(&self, db: &Database<Self>, key: &u32) -> Result<u32, QueryError> {
        if *key == 0 {
            return Ok(10);
        }
        let below = db.get(&(key - 1))?;
        if self.explode.get() {
            panic!("compute failed");
        }
        Ok(below + 1)
    }
}

#[test]
fn test_panic_in_compute_leaves_no_false_cycle() {
    let db = Database::new(Panicky {
        explode: Cell::new(true),
    });
    let caught = catch_unwind(AssertUnwindSafe(|| db.get(&3)));
    assert!(caught.is_err());

    db.system().explode.set(false);
    assert_eq!(db.get(&3), Ok(13));
    assert_eq!(db.get(&2), Ok(12));
}

/// Each key reads the one below it.
struct Chain;

impl System for Chain {
    type Key = u32;
    type Value = u64;
    fn compute(&self, db: &Database<Self>, key: &u32) -> Result<u64, QueryError> {
        if *key == 0 {
            Ok(0)
        } else {
            Ok(db.get(&(key - 1))? + 1)
        }
    }
}

#[test]
fn test_depth_limit_refuses_a_deep_chain() {
    on_small_stack(|| {
        let db = Database::new(Chain).with_max_depth(200);
        assert_eq!(db.max_depth(), Some(200));
        assert_eq!(db.get(&100), Ok(100));
        assert_eq!(db.get(&100_000), Err(QueryError::TooDeep));
        // The failed resolution leaves the database usable.
        assert_eq!(db.get(&150), Ok(150));
    });
}

#[test]
fn test_depth_limit_counts_revalidation() {
    on_small_stack(|| {
        let mut db = Database::new(Chain).with_max_depth(64);
        db.set(0, 0);
        assert_eq!(db.get(&60), Ok(60));
        db.set(0, 1);
        assert_eq!(db.get(&60), Ok(61));
    });
}

#[test]
fn test_no_depth_limit_by_default() {
    let db = Database::new(Chain);
    assert_eq!(db.max_depth(), None);
    assert_eq!(db.get(&500), Ok(500));
}

#[test]
fn test_fallback_key_is_unaffected() {
    let db = Database::new(Recovering);
    assert_eq!(db.get(&K::Fallback), Ok(0));
}
