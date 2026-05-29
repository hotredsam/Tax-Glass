//! A single worksheet: a sparse grid of cells, each holding either a literal
//! value or a parsed formula.

use crate::address::{CellRange, CellRef};
use crate::condformat::Rule;
use crate::error::Result;
use crate::eval::evaluate_sheet;
use crate::formula::{self, Expr};
use crate::style::CellStyle;
use crate::value::Value;
use std::collections::HashMap;

/// Which axis a structural edit applies to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Axis {
    Row,
    Col,
}

/// A structural edit: inserting or deleting `count` rows/columns at index `at`.
#[derive(Debug, Clone, Copy)]
enum Edit {
    Insert { at: u32, count: u32 },
    Delete { at: u32, count: u32 },
}

impl Edit {
    /// Map an index on the edited axis to its new position, or `None` if the
    /// index falls inside a deleted span.
    fn map(&self, idx: u32) -> Option<u32> {
        match *self {
            Edit::Insert { at, count } => Some(if idx >= at { idx + count } else { idx }),
            Edit::Delete { at, count } => {
                if idx < at {
                    Some(idx)
                } else if idx < at + count {
                    None
                } else {
                    Some(idx - count)
                }
            }
        }
    }
}

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
    styles: HashMap<(u32, u32), CellStyle>,
    cond_rules: Vec<Rule>,
}

impl Sheet {
    /// Create an empty sheet with the given name.
    pub fn new(name: impl Into<String>) -> Self {
        Sheet {
            name: name.into(),
            cells: HashMap::new(),
            styles: HashMap::new(),
            cond_rules: Vec::new(),
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

    /// Set the visual style of a cell. Setting the default style clears it.
    pub fn set_style(&mut self, r: CellRef, style: CellStyle) {
        if style.is_default() {
            self.styles.remove(&(r.col, r.row));
        } else {
            self.styles.insert((r.col, r.row), style);
        }
    }

    /// The style of a cell, if one is set.
    pub fn style(&self, r: CellRef) -> Option<&CellStyle> {
        self.styles.get(&(r.col, r.row))
    }

    /// Mutably access a cell's style, inserting a default if absent. Useful for
    /// tweaking one attribute.
    pub fn style_mut(&mut self, r: CellRef) -> &mut CellStyle {
        self.styles.entry((r.col, r.row)).or_default()
    }

    /// Add a conditional-formatting rule (lower index = higher priority).
    pub fn add_conditional_rule(&mut self, rule: Rule) {
        self.cond_rules.push(rule);
    }

    /// The conditional-formatting rules, in priority order.
    pub fn conditional_rules(&self) -> &[Rule] {
        &self.cond_rules
    }

    /// Evaluate the sheet and resolve which conditional style (if any) applies
    /// to each cell. The first matching rule wins.
    pub fn conditional_styles(&self) -> HashMap<(u32, u32), CellStyle> {
        let computed = self.evaluate();
        crate::condformat::effective_styles(&self.cond_rules, &computed)
    }

    /// The displayed text of a cell: its computed value rendered through the
    /// cell's number format (or `General` if it has none).
    pub fn display(&self, r: CellRef) -> String {
        let value = self.get(r);
        match self.style(r).and_then(|s| s.number_format.as_deref()) {
            Some(code) => crate::format::format_value(&value, code),
            None => value.as_text(),
        }
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

    /// Insert `count` blank rows at row index `at` (zero-based), shifting cells
    /// below down and fixing up same-sheet references in every formula.
    pub fn insert_rows(&mut self, at: u32, count: u32) {
        self.apply_structural(Axis::Row, Edit::Insert { at, count });
    }

    /// Delete `count` rows starting at row index `at`. Cells in the deleted band
    /// are removed; references to them become `#REF!`; cells below shift up.
    pub fn delete_rows(&mut self, at: u32, count: u32) {
        self.apply_structural(Axis::Row, Edit::Delete { at, count });
    }

    /// Insert `count` blank columns at column index `at`.
    pub fn insert_cols(&mut self, at: u32, count: u32) {
        self.apply_structural(Axis::Col, Edit::Insert { at, count });
    }

    /// Delete `count` columns starting at column index `at`.
    pub fn delete_cols(&mut self, at: u32, count: u32) {
        self.apply_structural(Axis::Col, Edit::Delete { at, count });
    }

    /// Reposition every cell for a structural edit and rewrite formula
    /// references, regenerating each affected formula's source text.
    fn apply_structural(&mut self, axis: Axis, edit: Edit) {
        let (Edit::Insert { count, .. } | Edit::Delete { count, .. }) = edit;
        if count == 0 {
            return;
        }
        let old = std::mem::take(&mut self.cells);
        for (key, content) in old {
            let Some(new_key) = reposition(key, axis, &edit) else {
                continue; // cell sat in a deleted band
            };
            let new_content = match content {
                CellContent::Formula { ast, .. } => {
                    let ast = adjust_expr(&ast, axis, &edit);
                    CellContent::Formula {
                        src: formula::unparse(&ast),
                        ast,
                    }
                }
                literal => literal,
            };
            self.cells.insert(new_key, new_content);
        }

        // Styles move with their cells (dropped if their cell was deleted).
        let old_styles = std::mem::take(&mut self.styles);
        for (key, style) in old_styles {
            if let Some(new_key) = reposition(key, axis, &edit) {
                self.styles.insert(new_key, style);
            }
        }
    }

    /// Copy a cell to another location, translating relative references like a
    /// spreadsheet copy/paste. Literals are copied verbatim; formulas have their
    /// relative refs shifted by `to - from` (absolute `$` parts stay fixed).
    /// Copying an empty source clears the destination.
    pub fn copy_cell(&mut self, from: CellRef, to: CellRef) {
        let dcol = to.col as i64 - from.col as i64;
        let drow = to.row as i64 - from.row as i64;
        match self.content(from.col, from.row).cloned() {
            None => self.set_value(to, Value::Empty),
            Some(CellContent::Literal(v)) => self.set_value(to, v),
            Some(CellContent::Formula { ast, .. }) => {
                let ast = formula::translate(&ast, dcol, drow);
                self.cells.insert(
                    (to.col, to.row),
                    CellContent::Formula {
                        src: formula::unparse(&ast),
                        ast,
                    },
                );
            }
        }
    }

    /// Fill `source` into every cell of `target`, translating references per
    /// destination — the common drag-to-fill / paste-to-range operation.
    pub fn fill(&mut self, source: CellRef, target: CellRange) {
        for dest in target.cells() {
            self.copy_cell(source, dest);
        }
    }

    /// Evaluate every cell and return a grid of computed values keyed by
    /// `(col, row)`. Empty cells are omitted from the map.
    pub fn evaluate(&self) -> HashMap<(u32, u32), Value> {
        evaluate_sheet(self)
    }

    /// Evaluate in iterative mode, letting intentional circular references
    /// converge. Formula cells start at 0 and are recomputed until the largest
    /// numeric change is below `epsilon` or `max_iterations` is hit. Use this
    /// instead of [`Sheet::evaluate`] when a model relies on feedback loops.
    pub fn evaluate_iterative(
        &self,
        max_iterations: u32,
        epsilon: f64,
    ) -> HashMap<(u32, u32), Value> {
        crate::eval::evaluate_sheet_iterative(self, max_iterations, epsilon)
    }

    /// Convenience: the computed value at a single reference.
    pub fn get(&self, r: CellRef) -> Value {
        self.evaluate()
            .get(&Self::key(r))
            .cloned()
            .unwrap_or(Value::Empty)
    }
}

/// Map a stored cell's coordinate through a structural edit, or `None` if the
/// cell sits in a deleted band.
fn reposition(key: (u32, u32), axis: Axis, edit: &Edit) -> Option<(u32, u32)> {
    let (col, row) = key;
    match axis {
        Axis::Row => Some((col, edit.map(row)?)),
        Axis::Col => Some((edit.map(col)?, row)),
    }
}

/// Adjust the index of a single reference on the edited axis. Returns `None`
/// when the reference points into a deleted band (→ `#REF!`).
fn adjust_cellref(r: CellRef, axis: Axis, edit: &Edit) -> Option<CellRef> {
    match axis {
        Axis::Row => Some(CellRef {
            row: edit.map(r.row)?,
            ..r
        }),
        Axis::Col => Some(CellRef {
            col: edit.map(r.col)?,
            ..r
        }),
    }
}

/// Index of a reference on the edited axis.
fn axis_index(r: &CellRef, axis: Axis) -> u32 {
    match axis {
        Axis::Row => r.row,
        Axis::Col => r.col,
    }
}

fn with_axis_index(mut r: CellRef, axis: Axis, idx: u32) -> CellRef {
    match axis {
        Axis::Row => r.row = idx,
        Axis::Col => r.col = idx,
    }
    r
}

/// Adjust a range through an edit. The whole range becomes `#REF!` only when it
/// is entirely inside a deleted band; otherwise deleted endpoints clamp to the
/// surviving boundary (the range shrinks), matching spreadsheet behavior.
fn adjust_range(range: CellRange, axis: Axis, edit: &Edit) -> Option<CellRange> {
    let start_idx = axis_index(&range.start, axis);
    let end_idx = axis_index(&range.end, axis);

    if let Edit::Delete { at, count } = *edit {
        let start_deleted = start_idx >= at && start_idx < at + count;
        let end_deleted = end_idx >= at && end_idx < at + count;
        if start_deleted && end_deleted {
            return None; // whole range gone
        }
        let new_start = if start_deleted {
            at // collapses to the first surviving cell
        } else {
            edit.map(start_idx)?
        };
        let new_end = if end_deleted {
            at.saturating_sub(1) // last surviving cell before the band
        } else {
            edit.map(end_idx)?
        };
        if new_start > new_end {
            return None;
        }
        return Some(CellRange {
            start: with_axis_index(range.start, axis, new_start),
            end: with_axis_index(range.end, axis, new_end),
        });
    }

    // Insert: both endpoints always survive.
    Some(CellRange {
        start: adjust_cellref(range.start, axis, edit)?,
        end: adjust_cellref(range.end, axis, edit)?,
    })
}

/// Rewrite every same-sheet reference in an expression for a structural edit.
/// Sheet-qualified references target other sheets and are left untouched.
fn adjust_expr(expr: &Expr, axis: Axis, edit: &Edit) -> Expr {
    match expr {
        Expr::Ref(r) => match adjust_cellref(*r, axis, edit) {
            Some(nr) => Expr::Ref(nr),
            None => Expr::RefError,
        },
        Expr::Range(range) => match adjust_range(*range, axis, edit) {
            Some(nr) => Expr::Range(nr),
            None => Expr::RefError,
        },
        Expr::Neg(inner) => Expr::Neg(Box::new(adjust_expr(inner, axis, edit))),
        Expr::Percent(inner) => Expr::Percent(Box::new(adjust_expr(inner, axis, edit))),
        Expr::Binary(op, a, b) => Expr::Binary(
            *op,
            Box::new(adjust_expr(a, axis, edit)),
            Box::new(adjust_expr(b, axis, edit)),
        ),
        Expr::Func(name, args) => Expr::Func(
            name.clone(),
            args.iter().map(|a| adjust_expr(a, axis, edit)).collect(),
        ),
        Expr::Array(rows) => Expr::Array(
            rows.iter()
                .map(|r| r.iter().map(|e| adjust_expr(e, axis, edit)).collect())
                .collect(),
        ),
        // A whole-column span tracks column inserts/deletes; whole-row tracks
        // row edits. On the other axis they are unaffected.
        Expr::ColSpan { start, end } if axis == Axis::Col => {
            match adjust_span(*start, *end, edit) {
                Some((s, e)) => Expr::ColSpan { start: s, end: e },
                None => Expr::RefError,
            }
        }
        Expr::RowSpan { start, end } if axis == Axis::Row => {
            match adjust_span(*start, *end, edit) {
                Some((s, e)) => Expr::RowSpan { start: s, end: e },
                None => Expr::RefError,
            }
        }
        // Literals, sheet-qualified refs, names, off-axis spans, and existing
        // #REF! pass through unchanged.
        other => other.clone(),
    }
}

/// Adjust a span's `start..=end` indices for an edit, clamping deleted
/// endpoints to the surviving boundary. `None` if the whole span is deleted.
fn adjust_span(start: u32, end: u32, edit: &Edit) -> Option<(u32, u32)> {
    match *edit {
        Edit::Insert { .. } => Some((edit.map(start)?, edit.map(end)?)),
        Edit::Delete { at, count } => {
            let start_deleted = start >= at && start < at + count;
            let end_deleted = end >= at && end < at + count;
            if start_deleted && end_deleted {
                return None;
            }
            let s = if start_deleted { at } else { edit.map(start)? };
            let e = if end_deleted {
                at.saturating_sub(1)
            } else {
                edit.map(end)?
            };
            if s > e {
                None
            } else {
                Some((s, e))
            }
        }
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

    #[test]
    fn insert_rows_shifts_cells_and_fixes_refs() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "10").unwrap();
        s.set_input(cell("A2"), "20").unwrap();
        s.set_formula(cell("A3"), "=A1+A2").unwrap();

        // Insert one row above row 2 (index 1): A2->A3, A3->A4.
        s.insert_rows(1, 1);
        assert_eq!(s.get(cell("A1")), Value::Number(10.0));
        assert_eq!(s.raw_text(cell("A2")), ""); // new blank row
        assert_eq!(s.get(cell("A3")), Value::Number(20.0));
        // The formula moved to A4 and its refs shifted to A1 and A3.
        assert_eq!(s.raw_text(cell("A4")), "=A1+A3");
        assert_eq!(s.get(cell("A4")), Value::Number(30.0));
    }

    #[test]
    fn delete_rows_breaks_refs_to_deleted_cells() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "5").unwrap();
        s.set_input(cell("A2"), "6").unwrap();
        s.set_formula(cell("B1"), "=A1+A2").unwrap();
        s.set_formula(cell("B2"), "=A2*10").unwrap();

        // Delete row 1 (index 0): A1 and B1 are removed; A2->A1, B2->B1.
        s.delete_rows(0, 1);
        assert_eq!(s.get(cell("A1")), Value::Number(6.0));
        // B2's formula moved to B1 and its A2 ref shifted up to A1.
        assert_eq!(s.raw_text(cell("B1")), "=A1*10");
        assert_eq!(s.get(cell("B1")), Value::Number(60.0));
    }

    #[test]
    fn deleting_a_referenced_cell_yields_ref_error() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_formula(cell("C1"), "=A1+1").unwrap();
        // Delete column A (index 0): A1 gone, C1->B1, ref A1 is broken.
        s.delete_cols(0, 1);
        assert_eq!(s.raw_text(cell("B1")), "=#REF!+1");
        assert_eq!(s.get(cell("B1")), Value::Error(CellError::Ref));
    }

    #[test]
    fn style_set_get_and_display() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1234.5").unwrap();
        s.style_mut(cell("A1")).number_format = Some("#,##0.00".into());
        s.style_mut(cell("A1")).font.bold = true;

        assert_eq!(s.display(cell("A1")), "1,234.50");
        assert!(s.style(cell("A1")).unwrap().font.bold);
        // A cell without a number format displays its general value.
        s.set_input(cell("A2"), "5").unwrap();
        assert_eq!(s.display(cell("A2")), "5");
    }

    #[test]
    fn conditional_formatting_highlights_matching_cells() {
        use crate::condformat::{Condition, Rule};
        use crate::style::{CellStyle, Color};

        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "5").unwrap();
        s.set_input(cell("A2"), "50").unwrap();
        s.set_formula(cell("A3"), "=A2*2").unwrap(); // 100

        let highlight = CellStyle {
            fill: Some(Color::rgb(255, 0, 0)),
            ..Default::default()
        };
        s.add_conditional_rule(Rule::new(
            CellRange::parse("A1:A3").unwrap(),
            Condition::GreaterThan(40.0),
            highlight.clone(),
        ));

        let styles = s.conditional_styles();
        assert!(!styles.contains_key(&(0, 0)), "5 not > 40");
        assert_eq!(styles.get(&(0, 1)), Some(&highlight)); // 50
        assert_eq!(styles.get(&(0, 2)), Some(&highlight)); // 100 (from formula)
    }

    #[test]
    fn styles_follow_cells_through_structural_edits() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A2"), "x").unwrap();
        s.style_mut(cell("A2")).font.italic = true;
        // Insert a row at the top: A2 (and its style) move to A3.
        s.insert_rows(0, 1);
        assert!(s.style(cell("A3")).unwrap().font.italic);
        assert!(s.style(cell("A2")).is_none());
    }

    #[test]
    fn copy_cell_translates_relative_refs() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_input(cell("A2"), "2").unwrap();
        s.set_input(cell("B1"), "10").unwrap();
        s.set_input(cell("B2"), "20").unwrap();
        s.set_formula(cell("A3"), "=A1+A2").unwrap();

        // Copy A3 -> B3: refs shift one column right.
        s.copy_cell(cell("A3"), cell("B3"));
        assert_eq!(s.raw_text(cell("B3")), "=B1+B2");
        assert_eq!(s.get(cell("B3")), Value::Number(30.0));
        // Original is unchanged.
        assert_eq!(s.raw_text(cell("A3")), "=A1+A2");
    }

    #[test]
    fn copy_keeps_absolute_refs_fixed() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "100").unwrap();
        s.set_formula(cell("B1"), "=$A$1*C1").unwrap();
        // Copy B1 -> B2: $A$1 stays, C1 -> C2.
        s.copy_cell(cell("B1"), cell("B2"));
        assert_eq!(s.raw_text(cell("B2")), "=$A$1*C2");
    }

    #[test]
    fn copy_off_the_grid_becomes_ref_error() {
        let mut s = Sheet::new("Sheet1");
        s.set_formula(cell("B1"), "=A1").unwrap();
        // Copy B1 -> A1: relative ref would point to column -1.
        s.copy_cell(cell("B1"), cell("A1"));
        assert_eq!(s.raw_text(cell("A1")), "=#REF!");
        assert_eq!(s.get(cell("A1")), Value::Error(CellError::Ref));
    }

    #[test]
    fn fill_propagates_a_formula_across_a_range() {
        let mut s = Sheet::new("Sheet1");
        for (i, v) in ["1", "2", "3"].iter().enumerate() {
            s.set_input(CellRef::new(0, i as u32), v).unwrap(); // A1:A3
            s.set_input(CellRef::new(1, i as u32), &((i + 1) * 10).to_string())
                .unwrap(); // B1:B3
        }
        s.set_formula(cell("C1"), "=A1*B1").unwrap();
        s.fill(cell("C1"), CellRange::parse("C1:C3").unwrap());
        assert_eq!(s.get(cell("C1")), Value::Number(10.0));
        assert_eq!(s.get(cell("C2")), Value::Number(40.0));
        assert_eq!(s.get(cell("C3")), Value::Number(90.0));
        assert_eq!(s.raw_text(cell("C3")), "=A3*B3");
    }

    #[test]
    fn insert_columns_expands_a_spanning_range() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_input(cell("B1"), "2").unwrap();
        s.set_input(cell("C1"), "3").unwrap();
        s.set_formula(cell("E1"), "=SUM(A1:C1)").unwrap();
        // Insert a column at index 1 (between A and B): range A1:C1 -> A1:D1.
        s.insert_cols(1, 1);
        assert_eq!(s.raw_text(cell("F1")), "=SUM(A1:D1)");
        assert_eq!(s.get(cell("F1")), Value::Number(6.0));
    }
}
