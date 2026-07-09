//! Criterion benchmarks for the three resolution paths — cold compute, cache
//! hit, and incremental recompute after an edit — plus early cutoff.
//!
//! The engine's own overhead is a `BTreeMap` lookup and a revision compare per
//! query; these benchmarks are dominated by that bookkeeping on purpose, since
//! the queries themselves do trivial arithmetic. The figures therefore measure
//! the framework, not a workload layered on top of it.

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use std::hint::black_box;

use query_lang::{Database, QueryError, System};

/// A linear chain: `Chain(0)` reads the input, `Chain(k)` reads `Chain(k-1)`.
/// Resolving `Chain(depth-1)` touches every link.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Chain {
    Input,
    Link(u32),
}

struct ChainSystem;

impl System for ChainSystem {
    type Key = Chain;
    type Value = i64;

    fn compute(&self, db: &Database<Self>, key: &Chain) -> Result<i64, QueryError> {
        match key {
            Chain::Input => Ok(0),
            Chain::Link(0) => Ok(db.get(&Chain::Input)?),
            Chain::Link(k) => Ok(db.get(&Chain::Link(k - 1))? + 1),
        }
    }
}

/// A wide sum: `Total` reads `Doubled(i)` for `i` in `0..width`, each of which
/// reads input `i`. Editing one input recomputes exactly one branch plus the sum.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Wide {
    Input(u32),
    Doubled(u32),
    Total,
}

struct WideSystem {
    width: u32,
}

impl System for WideSystem {
    type Key = Wide;
    type Value = i64;

    fn compute(&self, db: &Database<Self>, key: &Wide) -> Result<i64, QueryError> {
        match key {
            Wide::Input(_) => Ok(0),
            Wide::Doubled(i) => Ok(db.get(&Wide::Input(*i))? * 2),
            Wide::Total => {
                let mut total = 0;
                for i in 0..self.width {
                    total += db.get(&Wide::Doubled(i))?;
                }
                Ok(total)
            }
        }
    }
}

fn build_chain(depth: u32, input: i64) -> Database<ChainSystem> {
    let mut db = Database::new(ChainSystem);
    db.set(Chain::Input, input);
    let _ = db.get(&Chain::Link(depth - 1));
    db
}

fn bench_chain(c: &mut Criterion) {
    let mut group = c.benchmark_group("chain");
    for &depth in &[16u32, 256] {
        // Cold: fresh database, first resolution of the whole chain.
        group.bench_with_input(
            BenchmarkId::new("cold_build", depth),
            &depth,
            |b, &depth| {
                b.iter(|| {
                    let mut db = Database::new(ChainSystem);
                    db.set(Chain::Input, black_box(1));
                    black_box(db.get(&Chain::Link(depth - 1)).ok());
                });
            },
        );

        // Hit: re-resolve the tip of an already-built chain (all cache hits).
        group.bench_with_input(BenchmarkId::new("cache_hit", depth), &depth, |b, &depth| {
            let db = build_chain(depth, 1);
            b.iter(|| black_box(db.get(&Chain::Link(depth - 1)).ok()));
        });

        // Recompute: change the leaf input, re-resolve the tip (the whole chain
        // is invalidated and rebuilt).
        group.bench_with_input(
            BenchmarkId::new("edit_rebuild", depth),
            &depth,
            |b, &depth| {
                let mut db = build_chain(depth, 1);
                let mut v = 1i64;
                b.iter(|| {
                    v += 1;
                    db.set(Chain::Input, black_box(v));
                    black_box(db.get(&Chain::Link(depth - 1)).ok());
                });
            },
        );
    }
    group.finish();
}

fn bench_wide(c: &mut Criterion) {
    let mut group = c.benchmark_group("wide");
    let width = 256u32;

    // Edit one input in a 256-wide sum: only one branch and the total recompute,
    // the other 255 branches are validated by early cutoff.
    group.bench_function(BenchmarkId::new("edit_one_of", width), |b| {
        let mut db = Database::new(WideSystem { width });
        for i in 0..width {
            db.set(Wide::Input(i), i as i64);
        }
        let _ = db.get(&Wide::Total);
        let mut v = 0i64;
        b.iter(|| {
            v += 1;
            db.set(Wide::Input(black_box(0)), black_box(v));
            black_box(db.get(&Wide::Total).ok());
        });
    });

    group.finish();
}

criterion_group!(benches, bench_chain, bench_wide);
criterion_main!(benches);
