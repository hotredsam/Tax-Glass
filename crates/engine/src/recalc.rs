//! Incremental recalculation.
//!
//! [`RecalcEngine`] is a stateful façade over a [`Sheet`]: it owns the sheet, a
//! [`DependencyGraph`], a cache of computed values, and a dirty set. Editing a
//! cell marks only that cell and its transitive dependents dirty, so
//! `recalculate` recomputes the minimum needed rather than the whole sheet.
//! Clean precedents are reused from the cache, and the recursive evaluator
//! provides the correct evaluation order implicitly (no separate topo sort).
//!
//! The batch [`Sheet::evaluate`] path is unchanged; front-ends that edit cells
//! interactively use this engine for responsiveness.

use crate::address::CellRef;
use crate::deps::{CellKey, DependencyGraph};
use crate::error::Result;
use crate::eval::evaluate_targets;
use crate::sheet::Sheet;
use crate::value::Value;
use std::collections::{HashMap, HashSet};

/// A sheet plus the bookkeeping needed for incremental recompute.
#[derive(Debug, Clone)]
pub struct RecalcEngine {
    sheet: Sheet,
    graph: DependencyGraph,
    cache: HashMap<CellKey, Value>,
    dirty: HashSet<CellKey>,
}

impl RecalcEngine {
    /// Wrap a sheet. Every populated cell starts dirty, so the first
    /// `recalculate` computes the full sheet.
    pub fn new(sheet: Sheet) -> Self {
        let graph = DependencyGraph::build(&sheet);
        let dirty = sheet.iter().map(|(k, _)| *k).collect();
        RecalcEngine {
            sheet,
            graph,
            cache: HashMap::new(),
            dirty,
        }
    }

    /// Borrow the underlying sheet.
    pub fn sheet(&self) -> &Sheet {
        &self.sheet
    }

    /// Consume the engine and return the underlying sheet.
    pub fn into_sheet(self) -> Sheet {
        self.sheet
    }

    /// Number of cells currently pending recompute (for tests/diagnostics).
    pub fn dirty_len(&self) -> usize {
        self.dirty.len()
    }

    /// Set a cell from raw input (see [`Sheet::set_input`]), updating the graph
    /// and marking the affected cells dirty.
    pub fn set_input(&mut self, r: CellRef, raw: &str) -> Result<()> {
        self.sheet.set_input(r, raw)?;
        self.after_edit(r);
        Ok(())
    }

    /// Set a literal value at a cell.
    pub fn set_value(&mut self, r: CellRef, value: Value) {
        self.sheet.set_value(r, value);
        self.after_edit(r);
    }

    /// Set a formula at a cell.
    pub fn set_formula(&mut self, r: CellRef, src: impl Into<String>) -> Result<()> {
        self.sheet.set_formula(r, src)?;
        self.after_edit(r);
        Ok(())
    }

    /// Recompute all dirty cells, reusing clean cached values, then clear the
    /// dirty set.
    pub fn recalculate(&mut self) {
        if self.dirty.is_empty() {
            return;
        }
        // Reuse the existing cache as the clean seed: drop the dirty entries (so
        // they recompute) and let the evaluator borrow the rest — no O(n) copy.
        let mut cache = std::mem::take(&mut self.cache);
        for k in &self.dirty {
            cache.remove(k);
        }
        let updates = evaluate_targets(&self.sheet, &cache, &self.dirty);
        for (k, v) in updates {
            if v == Value::Empty {
                cache.remove(&k);
            } else {
                cache.insert(k, v);
            }
        }
        self.cache = cache;
        self.dirty.clear();
    }

    /// The computed value at a cell, recalculating first if anything is dirty.
    pub fn get(&mut self, r: CellRef) -> Value {
        self.recalculate();
        self.cache
            .get(&(r.col, r.row))
            .cloned()
            .unwrap_or(Value::Empty)
    }

    fn after_edit(&mut self, r: CellRef) {
        let key = (r.col, r.row);
        self.graph
            .update_cell(key, self.sheet.content(r.col, r.row));
        for affected in self.graph.affected(key) {
            self.dirty.insert(affected);
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
    fn computes_then_updates_incrementally() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "10").unwrap();
        s.set_formula(cell("A2"), "=A1*2").unwrap();
        s.set_formula(cell("A3"), "=A2+1").unwrap();

        let mut eng = RecalcEngine::new(s);
        assert_eq!(eng.get(cell("A3")), Value::Number(21.0));
        assert_eq!(eng.dirty_len(), 0, "clean after recalc");

        // Changing A1 dirties A1, A2, A3 (and nothing else).
        eng.set_value(cell("A1"), Value::Number(100.0));
        assert_eq!(eng.dirty_len(), 3);
        assert_eq!(eng.get(cell("A2")), Value::Number(200.0));
        assert_eq!(eng.get(cell("A3")), Value::Number(201.0));
    }

    #[test]
    fn unrelated_edit_has_minimal_dirty_set() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_formula(cell("A2"), "=A1+1").unwrap();
        s.set_input(cell("Z9"), "5").unwrap(); // independent cell

        let mut eng = RecalcEngine::new(s);
        eng.recalculate();
        eng.set_value(cell("Z9"), Value::Number(6.0));
        // Only Z9 is affected — A1/A2 stay clean.
        assert_eq!(eng.dirty_len(), 1);
        assert_eq!(eng.get(cell("A2")), Value::Number(2.0));
    }

    #[test]
    fn editing_a_formula_rewires_dependencies() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_input(cell("B1"), "100").unwrap();
        s.set_formula(cell("C1"), "=A1").unwrap();
        let mut eng = RecalcEngine::new(s);
        assert_eq!(eng.get(cell("C1")), Value::Number(1.0));

        eng.set_formula(cell("C1"), "=B1").unwrap();
        assert_eq!(eng.get(cell("C1")), Value::Number(100.0));

        // Now C1 no longer tracks A1: changing A1 leaves C1 alone.
        eng.set_value(cell("A1"), Value::Number(7.0));
        assert_eq!(eng.dirty_len(), 1, "only A1 dirty, not C1");
    }

    #[test]
    fn matches_batch_evaluation() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "3").unwrap();
        s.set_input(cell("A2"), "4").unwrap();
        s.set_formula(cell("A3"), "=SUM(A1:A2)").unwrap();
        s.set_formula(cell("A4"), "=A3*A3").unwrap();

        let batch = s.evaluate();
        let mut eng = RecalcEngine::new(s);
        eng.recalculate();
        for (&k, v) in &batch {
            assert_eq!(eng.get(CellRef::new(k.0, k.1)), *v);
        }
    }

    #[test]
    fn circular_reference_still_reported() {
        let mut eng = RecalcEngine::new(Sheet::new("Sheet1"));
        eng.set_formula(cell("A1"), "=A2").unwrap();
        eng.set_formula(cell("A2"), "=A1").unwrap();
        assert_eq!(eng.get(cell("A1")), Value::Error(CellError::Circular));
    }
}
