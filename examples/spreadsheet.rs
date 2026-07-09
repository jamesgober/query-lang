//! A dependency-tracked spreadsheet.
//!
//! Cells hold either a literal number (an input) or a formula that sums other
//! cells (a derived query). Editing one cell recomputes only the formulas that
//! transitively read it — exactly what a spreadsheet engine must do to stay
//! responsive on a large sheet.
//!
//! Run with:
//!
//! ```text
//! cargo run --example spreadsheet
//! ```

use query_lang::{Database, QueryError, System};

/// A cell address, `(column, row)`.
type Cell = (u8, u8);

/// The sheet's formulas, keyed by cell. A cell absent from this map is a plain
/// input; a cell present here is the sum of the listed cells.
struct Sheet {
    formulas: std::collections::BTreeMap<Cell, Vec<Cell>>,
    evals: std::cell::Cell<u64>,
}

impl System for Sheet {
    type Key = Cell;
    type Value = i64;

    fn compute(&self, db: &Database<Self>, cell: &Cell) -> Result<i64, QueryError> {
        match self.formulas.get(cell) {
            // A cell with no formula and no set value defaults to zero.
            None => Ok(0),
            Some(operands) => {
                self.evals.set(self.evals.get() + 1);
                let mut sum = 0;
                for operand in operands {
                    sum += db.get(operand)?;
                }
                Ok(sum)
            }
        }
    }
}

fn main() {
    let mut formulas = std::collections::BTreeMap::new();
    // C1 = A1 + A2 + A3   (a subtotal)
    formulas.insert((b'C', 1), vec![(b'A', 1), (b'A', 2), (b'A', 3)]);
    // C2 = B1 + B2        (another subtotal)
    formulas.insert((b'C', 2), vec![(b'B', 1), (b'B', 2)]);
    // D1 = C1 + C2        (the grand total)
    formulas.insert((b'D', 1), vec![(b'C', 1), (b'C', 2)]);

    let mut sheet = Database::new(Sheet {
        formulas,
        evals: std::cell::Cell::new(0),
    });

    // Seed the input cells.
    for (cell, value) in [
        ((b'A', 1), 10),
        ((b'A', 2), 20),
        ((b'A', 3), 30),
        ((b'B', 1), 5),
        ((b'B', 2), 7),
    ] {
        sheet.set(cell, value);
    }

    let total = resolve(&sheet, (b'D', 1));
    println!("initial D1 (grand total) = {total}");
    println!(
        "  formula evaluations so far: {}",
        sheet.system().evals.get()
    );

    // Edit a cell that only feeds the C1 subtotal.
    println!("\nediting A2: 20 -> 25");
    sheet.set((b'A', 2), 25);
    let total = resolve(&sheet, (b'D', 1));
    println!("D1 = {total}");
    println!(
        "  formula evaluations after edit: {} (only C1 and D1 recomputed; C2 was reused)",
        sheet.system().evals.get()
    );

    // Re-set a cell to the value it already holds: nothing recomputes.
    let before = sheet.system().evals.get();
    println!("\nre-setting B1 to its current value (no real change)");
    sheet.set((b'B', 1), 5);
    let total = resolve(&sheet, (b'D', 1));
    println!("D1 = {total}");
    println!(
        "  formula evaluations: {} (unchanged — every query was a cache hit)",
        sheet.system().evals.get()
    );
    debug_assert_eq!(sheet.system().evals.get(), before);

    println!("\ncache metrics: {}", sheet.stats());
}

/// Resolve a cell, treating a query cycle as a `#CYCLE!` sentinel rather than a
/// hard failure — the way a spreadsheet reports a circular reference.
fn resolve(sheet: &Database<Sheet>, cell: Cell) -> i64 {
    match sheet.get(&cell) {
        Ok(value) => value,
        Err(_) => {
            // The only resolution error today is a circular reference.
            println!("  #CYCLE! at {}{}", cell.0 as char, cell.1);
            0
        }
    }
}
