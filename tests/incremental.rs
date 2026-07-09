//! End-to-end behaviour of the query engine through its public API only.
//!
//! These tests drive a small stand-in for a compiler front end — source files as
//! inputs, per-file token counts and a whole-project total as derived queries —
//! and assert the two properties that make the engine worth using: an unrelated
//! edit recomputes nothing, and a value-preserving edit stops at the first query
//! whose result did not change (early cutoff).

#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::collections::BTreeMap;
use std::sync::Arc;

use query_lang::{Database, QueryError, System};

/// Which query a key names.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// Input: the text of file `n`.
    Source(u32),
    /// Derived: the number of whitespace-separated tokens in file `n`.
    Tokens(u32),
    /// Derived: the total token count across `files` given at construction.
    ProjectTotal,
}

/// A tiny "compiler" whose `compute` records how often each query kind ran, so a
/// test can assert precisely which queries recomputed after an edit.
struct Project {
    files: Vec<u32>,
    tokens_runs: Cell<u32>,
    total_runs: Cell<u32>,
}

impl Project {
    fn new(files: Vec<u32>) -> Self {
        Self {
            files,
            tokens_runs: Cell::new(0),
            total_runs: Cell::new(0),
        }
    }
}

impl System for Project {
    type Key = Key;
    // Wrapped in `Arc` so cloning a cached value bumps a refcount rather than
    // copying the string — the pattern the docs recommend for large values.
    type Value = Arc<String>;

    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<Arc<String>, QueryError> {
        match key {
            Key::Source(_) => Ok(Arc::new(String::new())), // default for an unset file
            Key::Tokens(n) => {
                self.tokens_runs.set(self.tokens_runs.get() + 1);
                let text = db.get(&Key::Source(*n))?;
                let count = text.split_whitespace().count();
                Ok(Arc::new(count.to_string()))
            }
            Key::ProjectTotal => {
                self.total_runs.set(self.total_runs.get() + 1);
                let mut total = 0usize;
                for &n in &self.files {
                    let tokens: usize = db.get(&Key::Tokens(n))?.parse().unwrap();
                    total += tokens;
                }
                Ok(Arc::new(total.to_string()))
            }
        }
    }
}

fn seed(db: &mut Database<Project>, files: &[(u32, &str)]) {
    for &(n, text) in files {
        db.set(Key::Source(n), Arc::new(text.to_string()));
    }
}

#[test]
fn test_project_total_is_sum_of_files() {
    let mut db = Database::new(Project::new(vec![0, 1, 2]));
    seed(&mut db, &[(0, "a b c"), (1, "one two"), (2, "single")]);
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "6"); // 3 + 2 + 1
}

#[test]
fn test_editing_one_file_recomputes_only_its_tokens() {
    let mut db = Database::new(Project::new(vec![0, 1, 2]));
    seed(&mut db, &[(0, "a b c"), (1, "one two"), (2, "single")]);
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "6");

    // Three files tokenized, one total computed.
    assert_eq!(db.system().tokens_runs.get(), 3);
    assert_eq!(db.system().total_runs.get(), 1);

    // Edit file 1 so its token count changes (2 -> 4).
    db.set(Key::Source(1), Arc::new("one two three four".to_string()));
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "8");

    // Only file 1 re-tokenized; files 0 and 2 were untouched. The total did
    // change, so it recomputed once more.
    assert_eq!(db.system().tokens_runs.get(), 4);
    assert_eq!(db.system().total_runs.get(), 2);
}

#[test]
fn test_value_preserving_edit_stops_at_early_cutoff() {
    let mut db = Database::new(Project::new(vec![0, 1]));
    seed(&mut db, &[(0, "a b c"), (1, "x y")]);
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "5");
    assert_eq!(db.system().total_runs.get(), 1);

    // Edit file 0 but keep the same token count (3 words -> 3 different words).
    db.set(Key::Source(0), Arc::new("d e f".to_string()));
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "5");

    // Tokens(0) recomputed (its source changed), but produced the same count, so
    // ProjectTotal was validated by early cutoff — it did NOT recompute.
    assert_eq!(db.system().tokens_runs.get(), 3); // was 2 after seed, +1 here
    assert_eq!(db.system().total_runs.get(), 1); // still 1
    assert!(db.stats().validated >= 1);
}

#[test]
fn test_no_op_edit_is_all_hits() {
    let mut db = Database::new(Project::new(vec![0, 1]));
    seed(&mut db, &[(0, "a b"), (1, "c")]);
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "3");
    let baseline = db.stats().computed;

    // Re-set both files to identical text: no revision change, no recomputation.
    db.set(Key::Source(0), Arc::new("a b".to_string()));
    db.set(Key::Source(1), Arc::new("c".to_string()));
    assert_eq!(db.get(&Key::ProjectTotal).unwrap().as_str(), "3");

    assert_eq!(db.stats().computed, baseline); // nothing recomputed
    assert!(db.stats().hits >= 1);
}

#[test]
fn test_repeated_queries_reuse_cache_across_the_graph() {
    let mut db = Database::new(Project::new(vec![0, 1, 2, 3]));
    seed(
        &mut db,
        &[(0, "a"), (1, "b b"), (2, "c c c"), (3, "d d d d")],
    );

    // Ask for every per-file query and the total; then ask again.
    let mut first = BTreeMap::new();
    for n in 0..4 {
        let _ = first.insert(n, db.get(&Key::Tokens(n)).unwrap());
    }
    let _ = db.get(&Key::ProjectTotal).unwrap();
    let computed_after_first = db.stats().computed;

    for n in 0..4 {
        assert_eq!(db.get(&Key::Tokens(n)).unwrap(), first[&n]);
    }
    let _ = db.get(&Key::ProjectTotal).unwrap();

    // The second sweep computed nothing new.
    assert_eq!(db.stats().computed, computed_after_first);
}

#[test]
fn test_cycle_surfaces_through_public_api() {
    // A self-referential system, resolved entirely through the public surface.
    struct Loop;
    impl System for Loop {
        type Key = u8;
        type Value = u8;
        fn compute(&self, db: &Database<Self>, k: &u8) -> Result<u8, QueryError> {
            db.get(k)
        }
    }
    let db = Database::new(Loop);
    assert_eq!(db.get(&0), Err(QueryError::Cycle));
}
