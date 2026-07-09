//! The incremental engine: input storage, memoization, and validation.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;
use core::cell::{Cell, RefCell};
use core::fmt;

use crate::error::QueryError;
use crate::revision::Revision;
use crate::stats::Stats;
use crate::system::System;

/// A stored input: a value a consumer placed into the database directly.
struct Input<V> {
    value: V,
    /// The revision at which this input last took a new value.
    changed_at: Revision,
}

/// The memo table: derived key to its cached result and dependency record.
type MemoMap<S> = BTreeMap<<S as System>::Key, Memo<<S as System>::Key, <S as System>::Value>>;

/// A memoized derived query: its cached value, what it read, and when.
struct Memo<K, V> {
    value: V,
    /// The queries this value was computed from, in read order. Re-examined to
    /// decide whether a stale memo can be reused.
    deps: Vec<K>,
    /// The revision at which `value` last became a *different* value. Early
    /// cutoff keeps this stamp old when a recomputation produces the same value.
    changed_at: Revision,
    /// The revision at which this memo was last confirmed current.
    verified_at: Revision,
}

/// An incremental query database: the store of inputs and the cache of derived
/// results, with automatic dependency tracking and invalidation.
///
/// This is the engine. A consumer defines its queries once by implementing
/// [`System`], hands the system to [`new`](Self::new), seeds the base facts with
/// [`set`](Self::set), and asks for results with [`get`](Self::get). Everything
/// between — remembering what each query read, noticing when an input makes a
/// cached result stale, recomputing only what actually changed — is the
/// database's job.
///
/// # How it stays correct and fast
///
/// The database holds a [`Revision`] clock that advances by one each time an
/// input takes a new value. Every cached query records two stamps: when it was
/// last *verified* against the clock, and when its value last *changed*. Asking
/// for a query takes one of three paths, counted in [`Stats`]:
///
/// - **Hit** — the query was already verified at the current revision; its value
///   is returned without touching its dependencies.
/// - **Validated** — the query is stale, but re-examining its recorded
///   dependencies shows none of them changed since the query was last verified,
///   so the cached value is reused. When a dependency *did* recompute but to the
///   same value, its change stamp stays old and dependents are validated rather
///   than recomputed. This *early cutoff* is what stops a local edit from
///   cascading through the whole graph.
/// - **Computed** — a genuine miss, or a dependency that truly changed; the
///   query's [`compute`](System::compute) runs and the new value is cached.
///
/// Because dependencies are recorded during computation rather than declared up
/// front, a query that branches on its inputs is tracked exactly: it depends on
/// what it actually read on the last run, and nothing more.
///
/// # Single-threaded by design
///
/// A `Database` is not `Sync`: query resolution walks a shared cache and a
/// dependency stack through interior mutability, which is correct and allocation-
/// light on one thread and carries no atomic overhead. Drive one database from
/// one thread; run independent databases on separate threads for parallelism.
///
/// # Examples
///
/// A three-layer computation — an input, a query over it, and a query over that
/// — recomputes only along the path an edit touches:
///
/// ```
/// use query_lang::{Database, System, QueryError};
///
/// #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
/// enum Key {
///     Width,       // an input
///     Doubled,     // = Width * 2
///     Labeled,     // = "w=" + Doubled
/// }
///
/// struct Layout;
/// impl System for Layout {
///     type Key = Key;
///     type Value = String;
///     fn compute(&self, db: &Database<Self>, key: &Key) -> Result<String, QueryError> {
///         match key {
///             Key::Width => Ok("0".into()),
///             Key::Doubled => {
///                 let w: i64 = db.get(&Key::Width)?.parse().unwrap_or(0);
///                 Ok((w * 2).to_string())
///             }
///             Key::Labeled => Ok(format!("w={}", db.get(&Key::Doubled)?)),
///         }
///     }
/// }
///
/// let mut db = Database::new(Layout);
/// db.set(Key::Width, "10".into());
/// assert_eq!(db.get(&Key::Labeled)?, "w=20");
/// assert_eq!(db.stats().computed, 2); // Width is a set input; Doubled and Labeled ran
///
/// // Re-ask without changing anything: a free hit, no recomputation.
/// assert_eq!(db.get(&Key::Labeled)?, "w=20");
/// assert_eq!(db.stats().hits, 1);
/// # Ok::<(), QueryError>(())
/// ```
pub struct Database<S: System> {
    system: S,
    revision: Revision,
    inputs: BTreeMap<S::Key, Input<S::Value>>,
    memos: RefCell<MemoMap<S>>,
    /// Dependency-collection stack. Each active `compute` pushes a frame; every
    /// [`get`](Self::get) call appends the key it read to the top frame.
    frames: RefCell<Vec<Vec<S::Key>>>,
    /// The keys whose `compute` is currently in progress, for cycle detection.
    active: RefCell<Vec<S::Key>>,
    computed: Cell<u64>,
    validated: Cell<u64>,
    hits: Cell<u64>,
}

impl<S: System> Database<S> {
    /// Create an empty database for the given query system.
    ///
    /// The database starts at the initial revision with no inputs and an empty
    /// cache. Seed inputs with [`set`](Self::set) before asking for derived
    /// queries.
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
    /// let db = Database::new(S);
    /// assert_eq!(db.stats().total(), 0);
    /// ```
    #[must_use]
    pub fn new(system: S) -> Self {
        Self {
            system,
            revision: Revision::START,
            inputs: BTreeMap::new(),
            memos: RefCell::new(BTreeMap::new()),
            frames: RefCell::new(Vec::new()),
            active: RefCell::new(Vec::new()),
            computed: Cell::new(0),
            validated: Cell::new(0),
            hits: Cell::new(0),
        }
    }

    /// Set an input to a value, advancing the revision if the value changed.
    ///
    /// This is the only way a value enters the database from outside. Once set, a
    /// key is an input: [`get`](Self::get) returns the stored value directly and
    /// [`compute`](System::compute) is never called for it. Setting the *same*
    /// value a key already holds is a no-op — the revision does not advance, and
    /// nothing that depends on the input is invalidated, so re-feeding unchanged
    /// facts costs nothing downstream. Setting a *different* value advances the
    /// revision, which is what later marks dependent queries stale.
    ///
    /// Setting a key that previously held a derived (computed) value promotes it
    /// to an input and discards the stale cached result.
    ///
    /// Taking `&mut self` is deliberate: mutating an input is the one operation
    /// that can invalidate cached results, so it is kept distinct from the shared
    /// `&self` reads that [`get`](Self::get) performs.
    ///
    /// # Examples
    ///
    /// ```
    /// use query_lang::{Database, System, QueryError};
    ///
    /// struct Echo;
    /// impl System for Echo {
    ///     type Key = u32;
    ///     type Value = i64;
    ///     fn compute(&self, db: &Database<Self>, k: &u32) -> Result<i64, QueryError> {
    ///         Ok(db.get(&(k + 1))? + 1) // reads input at k+1
    ///     }
    /// }
    ///
    /// let mut db = Database::new(Echo);
    /// db.set(1, 41);
    /// let r0 = db.revision();
    ///
    /// db.set(1, 41);              // same value
    /// assert_eq!(db.revision(), r0); // no change, clock still
    ///
    /// db.set(1, 99);              // new value
    /// assert!(db.revision() > r0);   // clock advanced
    /// ```
    pub fn set(&mut self, key: S::Key, value: S::Value) {
        // If this key cached a derived value, it is becoming an input; drop the
        // stale memo so the input and a leftover cache entry can never disagree.
        self.memos.get_mut().remove(&key);

        match self.inputs.get_mut(&key) {
            Some(existing) if existing.value == value => {
                // Same value: not a real change. Leave the revision untouched so
                // dependents stay valid.
            }
            Some(existing) => {
                self.revision = self.revision.next();
                existing.value = value;
                existing.changed_at = self.revision;
            }
            None => {
                self.revision = self.revision.next();
                let changed_at = self.revision;
                self.inputs.insert(key, Input { value, changed_at });
            }
        }
    }

    /// Resolve a query to its value, computing and caching it as needed.
    ///
    /// If `key` is a set input, its value is returned directly. Otherwise the
    /// query is derived: a valid cached value is reused (a hit or an early-cutoff
    /// validation), and only a real miss or a genuinely changed dependency runs
    /// [`compute`](System::compute). Call this both from application code and,
    /// from inside a `compute`, to read the queries a result depends on — reads
    /// through `get` are exactly what the engine records as dependencies.
    ///
    /// # Errors
    ///
    /// Returns [`QueryError::Cycle`] if resolving `key` requires a value that is
    /// still being computed further up the call chain — that is, if the query
    /// graph has a cycle.
    ///
    /// # Examples
    ///
    /// ```
    /// use query_lang::{Database, System, QueryError};
    ///
    /// struct Fib;
    /// impl System for Fib {
    ///     type Key = u64;
    ///     type Value = u64;
    ///     fn compute(&self, db: &Database<Self>, n: &u64) -> Result<u64, QueryError> {
    ///         // Memoized Fibonacci: each fib(n) is computed once and cached.
    ///         if *n < 2 { return Ok(*n); }
    ///         Ok(db.get(&(n - 1))?.wrapping_add(db.get(&(n - 2))?))
    ///     }
    /// }
    ///
    /// let db = Database::new(Fib);
    /// assert_eq!(db.get(&50)?, 12586269025);
    /// # Ok::<(), QueryError>(())
    /// ```
    pub fn get(&self, key: &S::Key) -> Result<S::Value, QueryError> {
        // Record this read as a dependency of the query currently computing, if
        // any. Reads made outside a `compute` (top-level queries) have no frame.
        if let Some(frame) = self.frames.borrow_mut().last_mut() {
            frame.push(key.clone());
        }
        let (value, _changed_at) = self.eval(key)?;
        Ok(value)
    }

    /// The current revision of the database.
    ///
    /// Advances by one each time [`set`](Self::set) gives an input a new value.
    /// Useful for asserting in tests that an operation did (or did not) change any
    /// input, and for correlating cache behaviour with input edits in logs.
    #[must_use]
    #[inline]
    pub const fn revision(&self) -> Revision {
        self.revision
    }

    /// A snapshot of the cumulative resolution counters.
    ///
    /// See [`Stats`] for what each counter means. Snapshot before and after an
    /// operation and subtract to measure exactly what that operation cost.
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
    /// let db = Database::new(S);
    /// let before = db.stats();
    /// let _ = db.get(&5)?;
    /// let after = db.stats();
    /// assert_eq!(after.computed - before.computed, 1);
    /// # Ok::<(), QueryError>(())
    /// ```
    #[must_use]
    pub fn stats(&self) -> Stats {
        Stats {
            computed: self.computed.get(),
            validated: self.validated.get(),
            hits: self.hits.get(),
        }
    }

    /// A shared reference to the query system backing this database.
    #[must_use]
    #[inline]
    pub const fn system(&self) -> &S {
        &self.system
    }

    /// Ensure `key` is current and return its value together with the revision at
    /// which that value last changed.
    ///
    /// This is the resolution core. Unlike [`get`](Self::get) it records no
    /// dependency — it is called both for the requested key and, during
    /// revalidation, for each recorded dependency, and only genuine reads through
    /// `get` should count as dependencies.
    fn eval(&self, key: &S::Key) -> Result<(S::Value, Revision), QueryError> {
        // Inputs are always current at their own change stamp.
        if let Some(input) = self.inputs.get(key) {
            return Ok((input.value.clone(), input.changed_at));
        }

        // Fast path: a memo already verified at the current revision.
        {
            let memos = self.memos.borrow();
            if let Some(memo) = memos.get(key) {
                if memo.verified_at == self.revision {
                    self.hits.set(self.hits.get().saturating_add(1));
                    return Ok((memo.value.clone(), memo.changed_at));
                }
            }
        }

        // Stale memo: snapshot what revalidation needs, then release the borrow —
        // checking dependencies recurses back through `eval`, which borrows again.
        let snapshot = self
            .memos
            .borrow()
            .get(key)
            .map(|m| (m.deps.clone(), m.verified_at, m.value.clone(), m.changed_at));

        if let Some((deps, verified_at, value, changed_at)) = snapshot {
            let mut inputs_changed = false;
            for dep in &deps {
                let (_dep_value, dep_changed_at) = self.eval(dep)?;
                if dep_changed_at > verified_at {
                    inputs_changed = true;
                    break;
                }
            }
            if !inputs_changed {
                // Early cutoff: nothing this query read actually changed. Reuse
                // the cached value; only its verification stamp moves forward.
                if let Some(memo) = self.memos.borrow_mut().get_mut(key) {
                    memo.verified_at = self.revision;
                }
                self.validated.set(self.validated.get().saturating_add(1));
                return Ok((value, changed_at));
            }
            return self.recompute(key, Some((value, changed_at)));
        }

        // No memo at all: compute from scratch.
        self.recompute(key, None)
    }

    /// Run `compute` for `key`, tracking the dependencies it reads and caching
    /// the result. `previous`, when present, is the memo's prior value and change
    /// stamp, used for early cutoff.
    fn recompute(
        &self,
        key: &S::Key,
        previous: Option<(S::Value, Revision)>,
    ) -> Result<(S::Value, Revision), QueryError> {
        // Cycle detection: re-entering a key already on the active stack means
        // the query graph is cyclic.
        if self.active.borrow().iter().any(|active| active == key) {
            return Err(QueryError::Cycle);
        }

        self.active.borrow_mut().push(key.clone());
        self.frames.borrow_mut().push(Vec::new());

        let result = self.system.compute(self, key);

        // Unwind the bookkeeping stacks whether compute succeeded or failed.
        let deps = self.frames.borrow_mut().pop().unwrap_or_default();
        self.active.borrow_mut().pop();

        let value = result?;

        // Early cutoff: if the recomputed value equals the old one, keep the old
        // change stamp so dependents see "unchanged" and are not recomputed.
        let changed_at = match previous {
            Some((old_value, old_changed_at)) if old_value == value => old_changed_at,
            _ => self.revision,
        };

        self.computed.set(self.computed.get().saturating_add(1));
        let memo = Memo {
            value: value.clone(),
            deps,
            changed_at,
            verified_at: self.revision,
        };
        self.memos.borrow_mut().insert(key.clone(), memo);

        Ok((value, changed_at))
    }
}

impl<S: System> fmt::Debug for Database<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Database")
            .field("revision", &self.revision)
            .field("inputs", &self.inputs.len())
            .field("memos", &self.memos.borrow().len())
            .field("stats", &self.stats())
            .finish()
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;
    use alloc::string::{String, ToString};
    use core::cell::Cell as StdCell;

    /// A system whose `compute` counts how many times each key ran, so tests can
    /// assert exactly which queries recomputed.
    struct Counting {
        // key -> number of times computed
        runs: StdCell<[u32; 8]>,
    }

    impl Counting {
        fn new() -> Self {
            Self {
                runs: StdCell::new([0; 8]),
            }
        }

        fn runs_of(&self, key: usize) -> u32 {
            self.runs.get()[key]
        }

        fn bump(&self, key: usize) {
            let mut r = self.runs.get();
            r[key] += 1;
            self.runs.set(r);
        }
    }

    // Key layout: 0 = input A, 1 = input B, 2 = A*10, 3 = (A*10)+(B), 4 = sign of 2
    impl System for Counting {
        type Key = usize;
        type Value = i64;
        fn compute(&self, db: &Database<Self>, key: &usize) -> Result<i64, QueryError> {
            self.bump(*key);
            match key {
                0 | 1 => Ok(0), // default when the input was never set
                2 => Ok(db.get(&0)? * 10),
                3 => Ok(db.get(&2)? + db.get(&1)?),
                4 => Ok(db.get(&2)?.signum()),
                _ => Ok(0),
            }
        }
    }

    #[test]
    fn test_input_get_returns_set_value() {
        let mut db = Database::new(Counting::new());
        db.set(0, 7);
        assert_eq!(db.get(&0).unwrap(), 7);
        // Inputs never invoke compute.
        assert_eq!(db.system().runs_of(0), 0);
    }

    #[test]
    fn test_first_get_computes_once() {
        let mut db = Database::new(Counting::new());
        db.set(0, 3);
        assert_eq!(db.get(&2).unwrap(), 30);
        assert_eq!(db.system().runs_of(2), 1);
        // Only key 2 computed; key 0 is a set input and never runs compute.
        assert_eq!(db.stats().computed, 1);
    }

    #[test]
    fn test_second_get_is_a_hit() {
        let mut db = Database::new(Counting::new());
        db.set(0, 3);
        assert_eq!(db.get(&2).unwrap(), 30);
        let before = db.stats();
        assert_eq!(db.get(&2).unwrap(), 30);
        let after = db.stats();
        assert_eq!(after.hits - before.hits, 1);
        assert_eq!(after.computed, before.computed); // no recompute
    }

    #[test]
    fn test_input_change_recomputes_dependents() {
        let mut db = Database::new(Counting::new());
        db.set(0, 3);
        assert_eq!(db.get(&2).unwrap(), 30);
        db.set(0, 4);
        assert_eq!(db.get(&2).unwrap(), 40);
        assert_eq!(db.system().runs_of(2), 2);
    }

    #[test]
    fn test_unchanged_input_set_does_not_recompute() {
        let mut db = Database::new(Counting::new());
        db.set(0, 3);
        assert_eq!(db.get(&2).unwrap(), 30);
        db.set(0, 3); // same value -> no revision bump
        assert_eq!(db.get(&2).unwrap(), 30);
        // Still verified at the current revision: a hit, not a recompute.
        assert_eq!(db.system().runs_of(2), 1);
    }

    #[test]
    fn test_multilayer_hit_after_no_change() {
        let mut db = Database::new(Counting::new());
        db.set(0, 3);
        // key 4 = signum(A*10); depends on key 2, which depends on input 0.
        assert_eq!(db.get(&4).unwrap(), 1);
        assert_eq!(db.system().runs_of(4), 1);
        assert_eq!(db.system().runs_of(2), 1);

        // Re-setting the input to the same value does not advance the revision,
        // so the next query is a plain hit and nothing recomputes.
        db.set(0, 3);
        assert_eq!(db.get(&4).unwrap(), 1);
        assert_eq!(db.system().runs_of(4), 1);
        assert_eq!(db.system().runs_of(2), 1);
    }

    #[test]
    fn test_early_cutoff_when_intermediate_value_is_stable() {
        // A system where an input change leaves an intermediate query's value
        // unchanged, so the top query is validated, not recomputed.
        struct AbsChain {
            top_runs: StdCell<u32>,
        }
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
        enum K {
            In,
            Abs,
            Plus1,
        }
        impl System for AbsChain {
            type Key = K;
            type Value = i64;
            fn compute(&self, db: &Database<Self>, key: &K) -> Result<i64, QueryError> {
                match key {
                    K::In => Ok(0),
                    K::Abs => Ok(db.get(&K::In)?.abs()),
                    K::Plus1 => {
                        self.top_runs.set(self.top_runs.get() + 1);
                        Ok(db.get(&K::Abs)? + 1)
                    }
                }
            }
        }

        let mut db = Database::new(AbsChain {
            top_runs: StdCell::new(0),
        });
        db.set(K::In, 5);
        assert_eq!(db.get(&K::Plus1).unwrap(), 6);
        assert_eq!(db.system().top_runs.get(), 1);

        // -5 and 5 have the same abs, so Abs recomputes to the same value and
        // Plus1 is validated (early cutoff), not recomputed.
        db.set(K::In, -5);
        assert_eq!(db.get(&K::Plus1).unwrap(), 6);
        assert_eq!(db.system().top_runs.get(), 1);
        assert!(db.stats().validated >= 1);
    }

    #[test]
    fn test_default_value_when_input_unset() {
        let db = Database::new(Counting::new());
        // Input 0 was never set; compute(0) returns its default of 0.
        assert_eq!(db.get(&2).unwrap(), 0);
    }

    #[test]
    fn test_direct_self_cycle_is_error() {
        struct SelfDep;
        impl System for SelfDep {
            type Key = u32;
            type Value = u32;
            fn compute(&self, db: &Database<Self>, k: &u32) -> Result<u32, QueryError> {
                db.get(k)
            }
        }
        let db = Database::new(SelfDep);
        assert_eq!(db.get(&1), Err(QueryError::Cycle));
    }

    #[test]
    fn test_indirect_cycle_is_error() {
        struct PingPong;
        impl System for PingPong {
            type Key = u32;
            type Value = u32;
            fn compute(&self, db: &Database<Self>, k: &u32) -> Result<u32, QueryError> {
                // 0 depends on 1, 1 depends on 0.
                match k {
                    0 => db.get(&1),
                    _ => db.get(&0),
                }
            }
        }
        let db = Database::new(PingPong);
        assert_eq!(db.get(&0), Err(QueryError::Cycle));
    }

    #[test]
    fn test_state_is_clean_after_cycle_error() {
        struct SelfDep;
        impl System for SelfDep {
            type Key = u32;
            type Value = u32;
            fn compute(&self, db: &Database<Self>, k: &u32) -> Result<u32, QueryError> {
                if *k == 0 { db.get(&0) } else { Ok(*k) }
            }
        }
        let db = Database::new(SelfDep);
        assert_eq!(db.get(&0), Err(QueryError::Cycle));
        // A non-cyclic query still resolves normally afterwards.
        assert_eq!(db.get(&9).unwrap(), 9);
    }

    #[test]
    fn test_set_promotes_derived_key_to_input() {
        let mut db = Database::new(Counting::new());
        db.set(0, 3);
        assert_eq!(db.get(&2).unwrap(), 30); // key 2 derived and cached
        db.set(2, 999); // promote key 2 to an input
        assert_eq!(db.get(&2).unwrap(), 999);
        // It is now an input: compute is never called for it again.
        let runs_before = db.system().runs_of(2);
        assert_eq!(db.get(&2).unwrap(), 999);
        assert_eq!(db.system().runs_of(2), runs_before);
    }

    #[test]
    fn test_branching_dependencies_tracked_precisely() {
        // A query reads one of two inputs depending on a selector input; only the
        // branch it actually took is a dependency.
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
        enum K {
            Select,
            Left,
            Right,
            Picked,
        }
        struct Switch {
            runs: StdCell<u32>,
        }
        impl System for Switch {
            type Key = K;
            type Value = i64;
            fn compute(&self, db: &Database<Self>, key: &K) -> Result<i64, QueryError> {
                match key {
                    K::Select | K::Left | K::Right => Ok(0),
                    K::Picked => {
                        self.runs.set(self.runs.get() + 1);
                        if db.get(&K::Select)? == 0 {
                            db.get(&K::Left)
                        } else {
                            db.get(&K::Right)
                        }
                    }
                }
            }
        }
        let mut db = Database::new(Switch {
            runs: StdCell::new(0),
        });
        db.set(K::Select, 0);
        db.set(K::Left, 100);
        db.set(K::Right, 200);
        assert_eq!(db.get(&K::Picked).unwrap(), 100);
        assert_eq!(db.system().runs.get(), 1);

        // Changing Right must NOT recompute Picked: it read Left, not Right.
        db.set(K::Right, 999);
        assert_eq!(db.get(&K::Picked).unwrap(), 100);
        assert_eq!(db.system().runs.get(), 1);

        // Changing Left DOES recompute Picked.
        db.set(K::Left, 111);
        assert_eq!(db.get(&K::Picked).unwrap(), 111);
        assert_eq!(db.system().runs.get(), 2);
    }

    #[test]
    fn test_string_values_work() {
        struct Greeter;
        impl System for Greeter {
            type Key = u32;
            type Value = String;
            fn compute(&self, db: &Database<Self>, k: &u32) -> Result<String, QueryError> {
                if *k == 0 {
                    Ok("world".to_string())
                } else {
                    Ok(alloc::format!("hello, {}", db.get(&0)?))
                }
            }
        }
        let db = Database::new(Greeter);
        assert_eq!(db.get(&1).unwrap(), "hello, world");
    }

    #[test]
    fn test_debug_reports_shape() {
        let mut db = Database::new(Counting::new());
        db.set(0, 1);
        let _ = db.get(&2).unwrap();
        let rendered = alloc::format!("{db:?}");
        assert!(rendered.contains("revision"));
        assert!(rendered.contains("memos"));
    }

    #[test]
    fn test_diamond_recomputes_shared_dep_once() {
        // top depends on left and right, both of which depend on a shared input.
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
        enum K {
            In,
            Left,
            Right,
            Top,
        }
        struct Diamond {
            in_runs: StdCell<u32>,
        }
        impl System for Diamond {
            type Key = K;
            type Value = i64;
            fn compute(&self, db: &Database<Self>, key: &K) -> Result<i64, QueryError> {
                match key {
                    K::In => Ok(0),
                    K::Left => Ok(db.get(&K::In)? + 1),
                    K::Right => Ok(db.get(&K::In)? + 2),
                    K::Top => {
                        self.in_runs.set(self.in_runs.get() + 1);
                        Ok(db.get(&K::Left)? + db.get(&K::Right)?)
                    }
                }
            }
        }
        let mut db = Database::new(Diamond {
            in_runs: StdCell::new(0),
        });
        db.set(K::In, 10);
        assert_eq!(db.get(&K::Top).unwrap(), (10 + 1) + (10 + 2));
        assert_eq!(db.system().in_runs.get(), 1);

        db.set(K::In, 20);
        assert_eq!(db.get(&K::Top).unwrap(), (20 + 1) + (20 + 2));
        assert_eq!(db.system().in_runs.get(), 2);
    }

    #[test]
    fn test_multiple_deps_each_invalidate() {
        // Guards against a regression where deps are dropped: a query with two
        // input deps must be invalidated by either.
        let mut db = Database::new(Counting::new());
        db.set(0, 1);
        db.set(1, 2);
        // key 3 = (A*10) + B = 10 + 2 = 12
        assert_eq!(db.get(&3).unwrap(), 12);
        db.set(1, 5);
        assert_eq!(db.get(&3).unwrap(), 15);
        // And the other dependency still invalidates it too.
        db.set(0, 2); // A*10 becomes 20
        assert_eq!(db.get(&3).unwrap(), 25);
    }
}
