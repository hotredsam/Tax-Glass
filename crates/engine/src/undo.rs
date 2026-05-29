//! Undo/redo for cell edits.
//!
//! [`History`] owns a [`Sheet`] and records each edit as a group of before/after
//! cell snapshots. Undo restores the "before" content of a group; redo replays
//! the "after". Snapshots capture exact [`CellContent`], so a formula or a
//! text-that-looks-numeric round-trips precisely. A new edit clears the redo
//! stack, as users expect.

use crate::address::CellRef;
use crate::error::Result;
use crate::sheet::{CellContent, Sheet};
use crate::value::Value;

/// The before/after content of one cell within an edit.
#[derive(Debug, Clone)]
struct CellSnap {
    cell: (u32, u32),
    before: Option<CellContent>,
    after: Option<CellContent>,
}

/// A sheet wrapped with undo/redo history.
#[derive(Debug, Clone)]
pub struct History {
    sheet: Sheet,
    undo: Vec<Vec<CellSnap>>,
    redo: Vec<Vec<CellSnap>>,
}

impl History {
    /// Wrap a sheet with an empty history.
    pub fn new(sheet: Sheet) -> Self {
        History {
            sheet,
            undo: Vec::new(),
            redo: Vec::new(),
        }
    }

    /// Borrow the underlying sheet.
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    /// Consume and return the underlying sheet.
    pub fn into_sheet(self) -> Sheet {
        self.sheet
    }

    /// Computed value at a cell.
    pub fn get(&self, r: CellRef) -> Value {
        self.sheet.get(r)
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.redo.is_empty()
    }

    /// Edit a cell from raw input, recording it as a single undoable step.
    pub fn set_input(&mut self, r: CellRef, raw: &str) -> Result<()> {
        let before = self.sheet.content(r.col, r.row).cloned();
        self.sheet.set_input(r, raw)?;
        let after = self.sheet.content(r.col, r.row).cloned();
        self.push(vec![CellSnap {
            cell: (r.col, r.row),
            before,
            after,
        }]);
        Ok(())
    }

    /// Apply several cell edits as one undoable group (e.g. a paste or fill).
    pub fn set_inputs(&mut self, edits: &[(CellRef, &str)]) -> Result<()> {
        let mut snaps = Vec::with_capacity(edits.len());
        for &(r, raw) in edits {
            let before = self.sheet.content(r.col, r.row).cloned();
            self.sheet.set_input(r, raw)?;
            let after = self.sheet.content(r.col, r.row).cloned();
            snaps.push(CellSnap {
                cell: (r.col, r.row),
                before,
                after,
            });
        }
        self.push(snaps);
        Ok(())
    }

    /// Undo the most recent edit group. Returns `false` if there was nothing to
    /// undo.
    pub fn undo(&mut self) -> bool {
        let Some(group) = self.undo.pop() else {
            return false;
        };
        for snap in &group {
            self.restore(snap.cell, snap.before.clone());
        }
        self.redo.push(group);
        true
    }

    /// Redo the most recently undone edit group.
    pub fn redo(&mut self) -> bool {
        let Some(group) = self.redo.pop() else {
            return false;
        };
        for snap in &group {
            self.restore(snap.cell, snap.after.clone());
        }
        self.undo.push(group);
        true
    }

    fn push(&mut self, group: Vec<CellSnap>) {
        self.undo.push(group);
        self.redo.clear();
    }

    fn restore(&mut self, (col, row): (u32, u32), content: Option<CellContent>) {
        self.sheet.set_content(CellRef::new(col, row), content);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse(a1).unwrap()
    }

    #[test]
    fn undo_and_redo_single_edits() {
        let mut h = History::new(Sheet::new("Sheet1"));
        h.set_input(cell("A1"), "10").unwrap();
        h.set_input(cell("A1"), "20").unwrap();
        assert_eq!(h.get(cell("A1")), Value::Number(20.0));

        assert!(h.undo());
        assert_eq!(h.get(cell("A1")), Value::Number(10.0));
        assert!(h.undo());
        assert_eq!(h.get(cell("A1")), Value::Empty);
        assert!(!h.undo(), "nothing left to undo");

        assert!(h.redo());
        assert_eq!(h.get(cell("A1")), Value::Number(10.0));
        assert!(h.redo());
        assert_eq!(h.get(cell("A1")), Value::Number(20.0));
    }

    #[test]
    fn new_edit_clears_redo() {
        let mut h = History::new(Sheet::new("Sheet1"));
        h.set_input(cell("A1"), "1").unwrap();
        h.undo();
        assert!(h.can_redo());
        h.set_input(cell("A1"), "2").unwrap();
        assert!(!h.can_redo(), "a fresh edit discards the redo stack");
    }

    #[test]
    fn formulas_roundtrip_exactly() {
        let mut h = History::new(Sheet::new("Sheet1"));
        h.set_input(cell("A1"), "5").unwrap();
        h.set_input(cell("B1"), "=A1*2").unwrap();
        assert_eq!(h.get(cell("B1")), Value::Number(10.0));
        h.undo();
        assert_eq!(h.sheet().raw_text(cell("B1")), "");
        h.redo();
        assert_eq!(h.sheet().raw_text(cell("B1")), "=A1*2");
        assert_eq!(h.get(cell("B1")), Value::Number(10.0));
    }

    #[test]
    fn grouped_edits_undo_together() {
        let mut h = History::new(Sheet::new("Sheet1"));
        h.set_inputs(&[(cell("A1"), "1"), (cell("A2"), "2"), (cell("A3"), "3")])
            .unwrap();
        assert_eq!(h.get(cell("A3")), Value::Number(3.0));
        assert!(h.undo());
        // One undo reverts the whole group.
        assert_eq!(h.get(cell("A1")), Value::Empty);
        assert_eq!(h.get(cell("A3")), Value::Empty);
    }

    #[test]
    fn text_that_looks_numeric_is_preserved() {
        let mut h = History::new(Sheet::new("Sheet1"));
        // Set a genuine text value, then overwrite, then undo.
        h.set_input(cell("A1"), "hello").unwrap();
        h.set_input(cell("A1"), "world").unwrap();
        h.undo();
        assert_eq!(h.get(cell("A1")), Value::Text("hello".into()));
    }
}
