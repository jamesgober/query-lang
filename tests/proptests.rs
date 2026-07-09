//! Property-based tests holding the engine to its core invariants over a wide
//! space of input-edit sequences.
//!
//! The engine's whole value rests on one promise: an incrementally maintained
//! result is always *exactly* the result a from-scratch computation would give.
//! Each property here drives the database through an arbitrary sequence of edits
//! and queries and checks it against a simple non-incremental oracle.

#![allow(clippy::unwrap_used)]

use std::cell::Cell;
use std::collections::BTreeMap;

use proptest::prelude::*;
use query_lang::{Database, QueryError, Revision, System};

/// Number of input slots the model uses.
const SLOTS: u32 = 6;

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Key {
    /// Input slot `i`.
    In(u32),
    /// Derived: `In(i) * 2`.
    Doubled(u32),
    /// Derived: the sum over all slots of `Doubled(i)`.
    Total,
}

/// Sum-of-doubles system with a recomputation counter for cost assertions.
struct Model {
    doubled_runs: Cell<u64>,
}

impl Model {
    fn new() -> Self {
        Self {
            doubled_runs: Cell::new(0),
        }
    }
}

impl System for Model {
    type Key = Key;
    type Value = i64;

    fn compute(&self, db: &Database<Self>, key: &Key) -> Result<i64, QueryError> {
        match key {
            Key::In(_) => Ok(0), // default for a slot never set
            Key::Doubled(i) => {
                self.doubled_runs.set(self.doubled_runs.get() + 1);
                Ok(db.get(&Key::In(*i))?.wrapping_mul(2))
            }
            Key::Total => {
                let mut total: i64 = 0;
                for i in 0..SLOTS {
                    total = total.wrapping_add(db.get(&Key::Doubled(i))?);
                }
                Ok(total)
            }
        }
    }
}

/// Non-incremental oracle: the total the engine must agree with, computed
/// directly from the current input model.
fn oracle_total(inputs: &BTreeMap<u32, i64>) -> i64 {
    (0..SLOTS)
        .map(|i| inputs.get(&i).copied().unwrap_or(0).wrapping_mul(2))
        .fold(0i64, i64::wrapping_add)
}

/// A single scripted step against the database.
#[derive(Debug, Clone)]
enum Step {
    Set(u32, i64),
    GetTotal,
    GetDoubled(u32),
}

fn step_strategy() -> impl Strategy<Value = Step> {
    prop_oneof![
        (0..SLOTS, any::<i64>()).prop_map(|(i, v)| Step::Set(i, v)),
        Just(Step::GetTotal),
        (0..SLOTS).prop_map(Step::GetDoubled),
    ]
}

proptest! {
    /// After any sequence of edits and queries, the incremental total always
    /// equals the oracle computed from the current inputs.
    #[test]
    fn prop_total_always_matches_oracle(steps in prop::collection::vec(step_strategy(), 0..80)) {
        let mut db = Database::new(Model::new());
        let mut inputs: BTreeMap<u32, i64> = BTreeMap::new();

        for step in steps {
            match step {
                Step::Set(i, v) => {
                    db.set(Key::In(i), v);
                    let _ = inputs.insert(i, v);
                }
                Step::GetTotal => {
                    prop_assert_eq!(db.get(&Key::Total).unwrap(), oracle_total(&inputs));
                }
                Step::GetDoubled(i) => {
                    let expected = inputs.get(&i).copied().unwrap_or(0).wrapping_mul(2);
                    prop_assert_eq!(db.get(&Key::Doubled(i)).unwrap(), expected);
                }
            }
        }

        // A final check regardless of what the script asked for.
        prop_assert_eq!(db.get(&Key::Total).unwrap(), oracle_total(&inputs));
    }

    /// The incremental database and a database built fresh from the final inputs
    /// agree on every query — incremental maintenance never diverges from a
    /// cold build.
    #[test]
    fn prop_incremental_matches_cold_build(edits in prop::collection::vec((0..SLOTS, any::<i64>()), 0..60)) {
        let mut incremental = Database::new(Model::new());
        let mut inputs: BTreeMap<u32, i64> = BTreeMap::new();
        for &(i, v) in &edits {
            incremental.set(Key::In(i), v);
            let _ = incremental.get(&Key::Total).unwrap(); // force intermediate maintenance
            let _ = inputs.insert(i, v);
        }

        let mut cold = Database::new(Model::new());
        for (&i, &v) in &inputs {
            cold.set(Key::In(i), v);
        }

        prop_assert_eq!(incremental.get(&Key::Total).unwrap(), cold.get(&Key::Total).unwrap());
        for i in 0..SLOTS {
            prop_assert_eq!(
                incremental.get(&Key::Doubled(i)).unwrap(),
                cold.get(&Key::Doubled(i)).unwrap()
            );
        }
    }

    /// The revision never runs backwards, and re-setting an input to its current
    /// value never advances it.
    #[test]
    fn prop_revision_is_monotonic(edits in prop::collection::vec((0..SLOTS, any::<i64>()), 0..60)) {
        let mut db = Database::new(Model::new());
        let mut inputs: BTreeMap<u32, i64> = BTreeMap::new();
        let mut prev: Revision = db.revision();

        for (i, v) in edits {
            let existed_same = inputs.get(&i) == Some(&v);
            db.set(Key::In(i), v);
            let now = db.revision();
            prop_assert!(now >= prev);
            if existed_same {
                prop_assert_eq!(now, prev); // no-op edit does not tick the clock
            }
            prev = now;
            let _ = inputs.insert(i, v);
        }
    }

    /// Setting an input to a value that leaves every doubled result unchanged
    /// (namely, the same value) triggers no recomputation of `Doubled`.
    #[test]
    fn prop_unchanged_input_never_recomputes(i in 0..SLOTS, v in any::<i64>(), repeats in 1u32..8) {
        let mut db = Database::new(Model::new());
        db.set(Key::In(i), v);
        let _ = db.get(&Key::Doubled(i)).unwrap();
        let runs_after_first = db.system().doubled_runs.get();

        for _ in 0..repeats {
            db.set(Key::In(i), v); // identical value each time
            let _ = db.get(&Key::Doubled(i)).unwrap();
        }
        prop_assert_eq!(db.system().doubled_runs.get(), runs_after_first);
    }
}
