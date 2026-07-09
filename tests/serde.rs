//! Serialization of the observability types behind the `serde` feature.
//!
//! Only [`Revision`](query_lang::Revision) and [`Stats`](query_lang::Stats)
//! derive `Serialize`; this file confirms the derivations produce the shape a
//! log or dashboard would consume. Compiled only when the feature is enabled.

#![cfg(feature = "serde")]
#![allow(clippy::unwrap_used)]

use query_lang::{Database, QueryError, Stats, System};

struct S;
impl System for S {
    type Key = u32;
    type Value = i64;
    fn compute(&self, db: &Database<Self>, k: &u32) -> Result<i64, QueryError> {
        if *k == 0 { Ok(0) } else { Ok(db.get(&0)? + 1) }
    }
}

#[test]
fn test_stats_serializes_to_object() {
    let stats = Stats {
        computed: 3,
        validated: 2,
        hits: 5,
    };
    let json = serde_json::to_value(stats).unwrap();
    assert_eq!(json["computed"], 3);
    assert_eq!(json["validated"], 2);
    assert_eq!(json["hits"], 5);
}

#[test]
fn test_revision_serializes_to_number() {
    let mut db = Database::new(S);
    db.set(0, 1);
    db.set(0, 2);
    let json = serde_json::to_value(db.revision()).unwrap();
    assert_eq!(json, serde_json::json!(2));
}

#[test]
fn test_live_stats_serialize() {
    let mut db = Database::new(S);
    db.set(0, 10);
    let _ = db.get(&1).unwrap(); // one computation
    let _ = db.get(&1).unwrap(); // one hit
    let json = serde_json::to_value(db.stats()).unwrap();
    assert_eq!(json["computed"], 1);
    assert_eq!(json["hits"], 1);
}
