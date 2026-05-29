//! A single worksheet: a sparse grid of cells, each holding either a literal
//! value or a parsed formula.

use crate::address::CellRef;
use crate::error::Result;
use crate::eval::evaluate_sheet;
use crate::formula::{self, Expr};
use crate::value::Value;
use std::collections::HashMap;

/// What a cell stores: a literal value or a formula (kept both as source text
/// and as its parsed AST so we can re-display and re-evaluate without reparsing).
#[derive(Debug, Clone, PartialEq)]
pub enum CellContent {
    Literal(Value),
    Formula { src: String, ast: Expr },
}

/// A worksheet. Cells are keyed by zero-based `(col, row)`; absolute markers on
/// references don't affect storage identity.
#[derive(Debug, Clone)]
pub struct Sheet {
    pub name: String,
    cells: HashMap<(u32, u32), CellContent>,
}

impl Sheet {
    /// Create an empty sheet with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Sheet {
            name: name.into(),
            cells: HashMap::new(),
        }
    }

    fn key(r: CellRef) -> (u32, u32) {
        (r.col, r.row)
    }

    /// Store a literal value at a cell. Setting [`Value::Empty`] clears it.
    pub fn set_value(&mut self, r: CellRef, value: Value) {
        if value == Value::Empty {
            self.cells.remove(&Self::key(r));
        } else {
            self.cells.insert(Self::key(r), CellContent::Literal(value));
        }
    }

    /// Store a formula at a cell from its source (with or without a leading `=`).
    pub fn set_formula(&mut self, r: CellRef, src: impl Into<String>) -> Result<()> {
        let src = src.into();
        let body = src.strip_prefix('=').unwrap_or(&src);
        let ast = formula::parse(body)?;
        self.cells.insert(
            Self::key(r),
            CellContent::Formula {
                src: body.to_string(),
                ast,
            },
        );
        Ok(())
    }

    /// Set a cell from raw user/import text, inferring the type:
    /// a leading `=` is a formula, `TRUE`/`FALSE` are booleans, otherwise we try
    /// a number and fall back to text. An empty string clears the cell.
    pub fn set_input(&mut self, r: CellRef, raw: &str) -> Result<()> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            self.set_value(r, Value::Empty);
            return Ok(());
        }
        if let Some(body) = trimmed.strip_prefix('=') {
            return self.set_formula(r, body);
        }
        match trimmed.to_ascii_uppercase().as_str() {
            "TRUE" => self.set_value(r, Value::Bool(true)),
            "FALSE" => self.set_value(r, Value::Bool(false)),
            _ => {
                if let Ok(n) = trimmed.parse::<f64>() {
                    self.set_value(r, Value::Number(n));
                } else {
                    self.set_value(r, Value::Text(raw.to_string()));
                }
            }
        }
        Ok(())
    }

    /// Borrow the content stored at a coordinate, if any.
    pub fn content(&self, col: u32, row: u32) -> Option<&CellContent> {
        self.cells.get(&(col, row))
    }

    /// The raw, editable text of a cell — what you'd type to recreate it.
    /// Formulas come back with a leading `=`.
    pub fn raw_text(&self, r: CellRef) -> String {
        match self.content(r.col, r.row) {
            None => String::new(),
            Some(CellContent::Literal(v)) => v.as_text(),
            Some(CellContent::Formula { src, .. }) => format!("={src}"),
        }
    }

    /// Number of populated cells.
    pub fn len(&self) -> usize {
        self.cells.len()
    }

    /// Whether the sheet has no populated cells.
    pub fn is_empty(&self) -> bool {
        self.cells.is_empty()
    }

    /// Iterate populated cells as `((col, row), content)`.
    pub fn iter(&self) -> impl Iterator<Item = (&(u32, u32), &CellContent)> {
        self.cells.iter()
    }

    /// The 1-past-the-end `(cols, rows)` extent needed to contain every
    /// populated cell. `(0, 0)` for an empty sheet.
    pub fn dimensions(&self) -> (u32, u32) {
        let mut cols = 0;
        let mut rows = 0;
        for &(c, r) in self.cells.keys() {
            cols = cols.max(c + 1);
            rows = rows.max(r + 1);
        }
        (cols, rows)
    }

    /// Evaluate every cell and return a grid of computed values keyed by
    /// `(col, row)`. Empty cells are omitted from the map.
    pub fn evaluate(&self) -> HashMap<(u32, u32), Value> {
        evaluate_sheet(self)
    }

    /// Convenience: the computed value at a single reference.
    pub fn get(&self, r: CellRef) -> Value {
        self.evaluate()
            .get(&Self::key(r))
            .cloned()
            .unwrap_or(Value::Empty)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::CellError;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse(a1).unwrap()
    }

    #[test]
    fn set_input_infers_types() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "42").unwrap();
        s.set_input(cell("A2"), "hello").unwrap();
        s.set_input(cell("A3"), "TRUE").unwrap();
        s.set_input(cell("A4"), "=1+1").unwrap();
        assert_eq!(s.get(cell("A1")), Value::Number(42.0));
        assert_eq!(s.get(cell("A2")), Value::Text("hello".into()));
        assert_eq!(s.get(cell("A3")), Value::Bool(true));
        assert_eq!(s.get(cell("A4")), Value::Number(2.0));
    }

    #[test]
    fn formulas_reference_other_cells() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "10").unwrap();
        s.set_input(cell("A2"), "20").unwrap();
        s.set_formula(cell("A3"), "=A1+A2").unwrap();
        assert_eq!(s.get(cell("A3")), Value::Number(30.0));
    }

    #[test]
    fn raw_text_roundtrips() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "=SUM(B1:B3)").unwrap();
        assert_eq!(s.raw_text(cell("A1")), "=SUM(B1:B3)");
        s.set_input(cell("A1"), "3.5").unwrap();
        assert_eq!(s.raw_text(cell("A1")), "3.5");
    }

    #[test]
    fn empty_input_clears_cell() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "5").unwrap();
        assert_eq!(s.len(), 1);
        s.set_input(cell("A1"), "").unwrap();
        assert_eq!(s.len(), 0);
    }

    #[test]
    fn circular_reference_is_detected() {
        let mut s = Sheet::new("Sheet1");
        s.set_formula(cell("A1"), "=A2").unwrap();
        s.set_formula(cell("A2"), "=A1").unwrap();
        assert_eq!(s.get(cell("A1")), Value::Error(CellError::Circular));
    }

    #[test]
    fn dimensions_track_extent() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_input(cell("C5"), "2").unwrap();
        assert_eq!(s.dimensions(), (3, 5));
    }
}
