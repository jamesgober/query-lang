//! A miniature compiler front end, to show early cutoff end to end.
//!
//! The pipeline is `source -> tokens -> symbol count -> report`. The interesting
//! property: an edit that changes the *text* but not the *tokens* (reformatting,
//! adding whitespace or a comment) recomputes the tokenizer, sees the same token
//! list, and stops there — the symbol count and the report are reused. That is
//! early cutoff, and it is what keeps an editor responsive as you type.
//!
//! Run with:
//!
//! ```text
//! cargo run --example build_pipeline
//! ```

use std::cell::Cell;
use std::sync::Arc;

use query_lang::{Database, QueryError, System};

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
enum Query {
    /// Input: the raw source text.
    Source,
    /// Derived: the source split into tokens (comments and whitespace dropped).
    Tokens,
    /// Derived: how many distinct identifiers the token stream mentions.
    SymbolCount,
    /// Derived: a one-line human-readable report.
    Report,
}

/// A stand-in front end that counts how often each stage runs.
#[derive(Default)]
struct FrontEnd {
    tokenize_runs: Cell<u64>,
    symbol_runs: Cell<u64>,
    report_runs: Cell<u64>,
}

/// The value type: every query yields a shared string payload. `Tokens` encodes
/// its list as newline-separated text so values stay comparable for early cutoff.
type Val = Arc<String>;

impl System for FrontEnd {
    type Key = Query;
    type Value = Val;

    fn compute(&self, db: &Database<Self>, key: &Query) -> Result<Val, QueryError> {
        match key {
            Query::Source => Ok(Arc::new(String::new())),
            Query::Tokens => {
                self.tokenize_runs.set(self.tokenize_runs.get() + 1);
                let source = db.get(&Query::Source)?;
                let tokens = tokenize(&source);
                Ok(Arc::new(tokens.join("\n")))
            }
            Query::SymbolCount => {
                self.symbol_runs.set(self.symbol_runs.get() + 1);
                let tokens = db.get(&Query::Tokens)?;
                let count = tokens
                    .lines()
                    .filter(|t| t.chars().next().is_some_and(char::is_alphabetic))
                    .collect::<std::collections::BTreeSet<_>>()
                    .len();
                Ok(Arc::new(count.to_string()))
            }
            Query::Report => {
                self.report_runs.set(self.report_runs.get() + 1);
                let symbols = db.get(&Query::SymbolCount)?;
                Ok(Arc::new(format!("{symbols} distinct identifier(s)")))
            }
        }
    }
}

/// Drop line comments (`// ...`) and split on non-identifier characters.
fn tokenize(source: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    for line in source.lines() {
        let code = line.split("//").next().unwrap_or("");
        let mut current = String::new();
        for ch in code.chars() {
            if ch.is_alphanumeric() || ch == '_' {
                current.push(ch);
            } else if !current.is_empty() {
                tokens.push(std::mem::take(&mut current));
            }
        }
        if !current.is_empty() {
            tokens.push(current);
        }
    }
    tokens
}

fn main() {
    let mut db = Database::new(FrontEnd::default());

    db.set(Query::Source, Arc::new("let x = add(x, y)".to_string()));
    println!("report: {}", get(&db, Query::Report));
    print_runs(&db, "after first build");

    // Reformat: extra spaces and a trailing comment. Same tokens, so the symbol
    // count and report are reused — only the tokenizer runs again.
    println!("\nedit: reformat + add a comment (tokens unchanged)");
    db.set(
        Query::Source,
        Arc::new("let   x = add( x , y )   // a note".to_string()),
    );
    println!("report: {}", get(&db, Query::Report));
    print_runs(&db, "after reformat");

    // A real change: introduce a new identifier. Now everything downstream reruns.
    println!("\nedit: introduce a new identifier `z`");
    db.set(Query::Source, Arc::new("let x = add(x, y, z)".to_string()));
    println!("report: {}", get(&db, Query::Report));
    print_runs(&db, "after real change");

    println!("\ncache metrics: {}", db.stats());
}

fn get(db: &Database<FrontEnd>, key: Query) -> String {
    match db.get(&key) {
        Ok(value) => value.as_str().to_string(),
        Err(_) => "<cycle>".to_string(), // the only resolution error is a cycle
    }
}

fn print_runs(db: &Database<FrontEnd>, label: &str) {
    let s = db.system();
    println!(
        "  {label}: tokenize={}, symbol_count={}, report={}",
        s.tokenize_runs.get(),
        s.symbol_runs.get(),
        s.report_runs.get()
    );
}
