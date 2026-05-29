//! # GlassSheet Engine
//!
//! The spreadsheet core behind GlassSheet: a cell/value model, A1-style
//! addressing, a formula parser, and a memoized evaluator with circular-
//! reference detection. Front-ends (CLI, desktop GUI, web) build presentation
//! and file I/O on top of this crate; the calculation logic lives here and is
//! heavily unit-tested.
//!
//! ```
//! use glasssheet_engine::{Sheet, CellRef, Value};
//!
//! let mut sheet = Sheet::new("Sheet1");
//! sheet.set_input(CellRef::parse("A1").unwrap(), "10").unwrap();
//! sheet.set_input(CellRef::parse("A2").unwrap(), "20").unwrap();
//! sheet.set_formula(CellRef::parse("A3").unwrap(), "=A1+A2").unwrap();
//! assert_eq!(sheet.get(CellRef::parse("A3").unwrap()), Value::Number(30.0));
//! ```

pub mod address;
pub mod condformat;
pub mod datetime;
pub mod deps;
pub mod error;
pub mod eval;
pub mod format;
pub mod formula;
pub mod recalc;
pub mod sheet;
pub mod style;
pub mod value;
pub mod workbook;

pub use address::{CellRange, CellRef};
pub use condformat::{Condition, Rule};
pub use deps::{CellKey, DependencyGraph};
pub use error::{EngineError, Result};
pub use eval::{evaluate_sheet_iterative, evaluate_workbook, Cells};
pub use format::format_value;
pub use formula::{BinOp, Expr};
pub use recalc::RecalcEngine;
pub use sheet::{CellContent, Sheet};
pub use style::{Alignment, Border, BorderStyle, Borders, CellStyle, Color, Font, HAlign, VAlign};
pub use value::{CellError, Value};
pub use workbook::{DefinedName, Workbook};
