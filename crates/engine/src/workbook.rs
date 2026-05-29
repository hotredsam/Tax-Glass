//! A workbook: an ordered collection of named worksheets.
//!
//! Sheet names are unique case-insensitively (as in Excel) and the insertion
//! order is preserved so tab order is stable. Cross-sheet formula references
//! are not resolved yet — each sheet still evaluates independently — but this
//! type is the container the front-ends and the native file format build on.

use crate::error::{EngineError, Result};
use crate::sheet::Sheet;
use crate::value::Value;
use std::collections::HashMap;

/// An ordered set of worksheets with a tracked active sheet.
#[derive(Debug, Clone)]
pub struct Workbook {
    sheets: Vec<Sheet>,
    active: usize,
}

impl Default for Workbook {
    /// A new workbook with a single empty sheet named `Sheet1`.
    fn default() -> Self {
        Workbook {
            sheets: vec![Sheet::new("Sheet1")],
            active: 0,
        }
    }
}

impl Workbook {
    /// A workbook with one empty `Sheet1` (same as [`Default`]).
    pub fn new() -> Self {
        Self::default()
    }

    /// An empty workbook with no sheets. Most callers want [`Workbook::new`];
    /// this is for builders that add every sheet explicitly.
    pub fn empty() -> Self {
        Workbook {
            sheets: Vec::new(),
            active: 0,
        }
    }

    /// Number of sheets.
    pub fn len(&self) -> usize {
        self.sheets.len()
    }

    /// Whether the workbook has no sheets.
    pub fn is_empty(&self) -> bool {
        self.sheets.is_empty()
    }

    /// Sheet names in tab order.
    pub fn sheet_names(&self) -> Vec<String> {
        self.sheets.iter().map(|s| s.name.clone()).collect()
    }

    /// Find the index of a sheet by name (case-insensitive).
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.sheets
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Borrow a sheet by name (case-insensitive).
    pub fn sheet(&self, name: &str) -> Option<&Sheet> {
        self.index_of(name).map(|i| &self.sheets[i])
    }

    /// Mutably borrow a sheet by name (case-insensitive).
    pub fn sheet_mut(&mut self, name: &str) -> Option<&mut Sheet> {
        self.index_of(name).map(|i| &mut self.sheets[i])
    }

    /// Borrow a sheet by tab index.
    pub fn sheet_at(&self, index: usize) -> Option<&Sheet> {
        self.sheets.get(index)
    }

    /// Mutably borrow a sheet by tab index.
    pub fn sheet_at_mut(&mut self, index: usize) -> Option<&mut Sheet> {
        self.sheets.get_mut(index)
    }

    /// All sheets in tab order.
    pub fn sheets(&self) -> &[Sheet] {
        &self.sheets
    }

    /// The active sheet's index.
    pub fn active_index(&self) -> usize {
        self.active
    }

    /// Borrow the active sheet, if the workbook is non-empty.
    pub fn active_sheet(&self) -> Option<&Sheet> {
        self.sheets.get(self.active)
    }

    /// Set the active sheet by index.
    pub fn set_active(&mut self, index: usize) -> Result<()> {
        if index < self.sheets.len() {
            self.active = index;
            Ok(())
        } else {
            Err(EngineError::BadReference(format!("sheet index {index}")))
        }
    }

    /// Add a new empty sheet, returning its index. Errors if the name collides
    /// (case-insensitively) with an existing sheet.
    pub fn add_sheet(&mut self, name: impl Into<String>) -> Result<usize> {
        let name = name.into();
        self.ensure_unique(&name, None)?;
        self.sheets.push(Sheet::new(name));
        Ok(self.sheets.len() - 1)
    }

    /// Insert an already-built sheet at the end. Errors on a name collision.
    pub fn push_sheet(&mut self, sheet: Sheet) -> Result<usize> {
        self.ensure_unique(&sheet.name, None)?;
        self.sheets.push(sheet);
        Ok(self.sheets.len() - 1)
    }

    /// Rename a sheet (by current name). Errors if the source is missing or the
    /// target name collides with a *different* sheet. Renaming to the same name
    /// (case change only) is allowed.
    pub fn rename_sheet(&mut self, current: &str, new_name: impl Into<String>) -> Result<()> {
        let new_name = new_name.into();
        let idx = self
            .index_of(current)
            .ok_or_else(|| EngineError::BadReference(format!("no sheet {current:?}")))?;
        self.ensure_unique(&new_name, Some(idx))?;
        self.sheets[idx].name = new_name;
        Ok(())
    }

    /// Remove a sheet by name. The active index is clamped to stay in range.
    /// Errors if the sheet is missing.
    pub fn remove_sheet(&mut self, name: &str) -> Result<Sheet> {
        let idx = self
            .index_of(name)
            .ok_or_else(|| EngineError::BadReference(format!("no sheet {name:?}")))?;
        let removed = self.sheets.remove(idx);
        if self.active >= self.sheets.len() {
            self.active = self.sheets.len().saturating_sub(1);
        } else if self.active > idx {
            self.active -= 1;
        }
        Ok(removed)
    }

    /// Move a sheet from one tab position to another, shifting the rest.
    pub fn move_sheet(&mut self, from: usize, to: usize) -> Result<()> {
        if from >= self.sheets.len() || to >= self.sheets.len() {
            return Err(EngineError::BadReference(format!(
                "move {from} -> {to} out of range"
            )));
        }
        let sheet = self.sheets.remove(from);
        self.sheets.insert(to, sheet);
        Ok(())
    }

    /// Evaluate every sheet, returning each sheet's computed grid keyed by
    /// sheet name. (Cross-sheet references arrive in a later change.)
    pub fn evaluate(&self) -> HashMap<String, HashMap<(u32, u32), Value>> {
        self.sheets
            .iter()
            .map(|s| (s.name.clone(), s.evaluate()))
            .collect()
    }

    fn ensure_unique(&self, name: &str, ignore: Option<usize>) -> Result<()> {
        if name.trim().is_empty() {
            return Err(EngineError::BadReference("empty sheet name".into()));
        }
        if let Some(existing) = self.index_of(name) {
            if Some(existing) != ignore {
                return Err(EngineError::BadReference(format!(
                    "duplicate sheet name {name:?}"
                )));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::CellRef;

    #[test]
    fn default_has_one_sheet() {
        let wb = Workbook::new();
        assert_eq!(wb.len(), 1);
        assert_eq!(wb.sheet_names(), ["Sheet1"]);
        assert_eq!(wb.active_index(), 0);
    }

    #[test]
    fn add_and_lookup_sheets() {
        let mut wb = Workbook::empty();
        wb.add_sheet("Budget").unwrap();
        wb.add_sheet("Notes").unwrap();
        assert_eq!(wb.sheet_names(), ["Budget", "Notes"]);
        assert!(wb.sheet("budget").is_some(), "lookup is case-insensitive");
        assert_eq!(wb.index_of("NOTES"), Some(1));
    }

    #[test]
    fn duplicate_names_are_rejected() {
        let mut wb = Workbook::new();
        let err = wb.add_sheet("sheet1").unwrap_err();
        assert!(matches!(err, EngineError::BadReference(_)));
    }

    #[test]
    fn rename_allows_case_change_but_not_collision() {
        let mut wb = Workbook::empty();
        wb.add_sheet("A").unwrap();
        wb.add_sheet("B").unwrap();
        wb.rename_sheet("A", "a").unwrap(); // case-only change is fine
        assert!(wb.rename_sheet("a", "B").is_err()); // collides with the other
        assert_eq!(wb.sheet_names(), ["a", "B"]);
    }

    #[test]
    fn remove_clamps_active() {
        let mut wb = Workbook::empty();
        for n in ["A", "B", "C"] {
            wb.add_sheet(n).unwrap();
        }
        wb.set_active(2).unwrap();
        wb.remove_sheet("C").unwrap();
        assert_eq!(wb.sheet_names(), ["A", "B"]);
        assert_eq!(wb.active_index(), 1, "active clamped into range");
    }

    #[test]
    fn move_sheet_reorders() {
        let mut wb = Workbook::empty();
        for n in ["A", "B", "C"] {
            wb.add_sheet(n).unwrap();
        }
        wb.move_sheet(0, 2).unwrap();
        assert_eq!(wb.sheet_names(), ["B", "C", "A"]);
    }

    #[test]
    fn evaluate_covers_every_sheet() {
        let mut wb = Workbook::empty();
        wb.add_sheet("One").unwrap();
        wb.sheet_mut("One")
            .unwrap()
            .set_formula(CellRef::parse("A1").unwrap(), "=2+3")
            .unwrap();
        wb.add_sheet("Two").unwrap();

        let all = wb.evaluate();
        assert_eq!(all.len(), 2);
        assert_eq!(all["One"][&(0, 0)], Value::Number(5.0));
        assert!(all.contains_key("Two"));
    }
}
