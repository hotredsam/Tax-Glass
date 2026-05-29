//! Dependency graph for a sheet's formulas.
//!
//! For every formula cell we record the set of cells it directly references
//! (its *precedents*) and the inverse (its *dependents*). This drives
//! dirty-tracking: when a cell changes, the cells that must be recomputed are
//! that cell plus everything transitively downstream of it. The incremental
//! recalculator ([`crate::recalc`]) consumes this graph.
//!
//! Only same-sheet references participate in the local graph; sheet-qualified
//! references (`Sheet2!A1`) are recorded at the workbook level elsewhere.

use crate::address::CellRef;
use crate::formula::Expr;
use crate::sheet::{CellContent, Sheet};
use std::collections::{HashMap, HashSet, VecDeque};

/// Zero-based `(col, row)` cell key.
pub type CellKey = (u32, u32);

/// Forward (dependents) and reverse (precedents) edges between cells.
#[derive(Debug, Default, Clone)]
pub struct DependencyGraph {
    precedents: HashMap<CellKey, HashSet<CellKey>>,
    dependents: HashMap<CellKey, HashSet<CellKey>>,
}

impl DependencyGraph {
    /// Build the graph for every formula in a sheet.
    pub fn build(sheet: &Sheet) -> Self {
        let mut graph = DependencyGraph::default();
        for (&key, content) in sheet.iter() {
            if let CellContent::Formula { ast, .. } = content {
                graph.set_precedents(key, refs_of(ast));
            }
        }
        graph
    }

    /// Replace the precedent set for one cell (re-extracted from its formula),
    /// keeping the reverse edges consistent. Pass an empty set for a literal or
    /// cleared cell.
    pub fn update_cell(&mut self, cell: CellKey, content: Option<&CellContent>) {
        let new_precs = match content {
            Some(CellContent::Formula { ast, .. }) => refs_of(ast),
            _ => HashSet::new(),
        };
        self.set_precedents(cell, new_precs);
    }

    fn set_precedents(&mut self, cell: CellKey, new_precs: HashSet<CellKey>) {
        // Drop stale reverse edges.
        if let Some(old) = self.precedents.remove(&cell) {
            for p in old {
                if let Some(deps) = self.dependents.get_mut(&p) {
                    deps.remove(&cell);
                    if deps.is_empty() {
                        self.dependents.remove(&p);
                    }
                }
            }
        }
        // Install new edges.
        if !new_precs.is_empty() {
            for &p in &new_precs {
                self.dependents.entry(p).or_default().insert(cell);
            }
            self.precedents.insert(cell, new_precs);
        }
    }

    /// Cells directly referenced by `cell`.
    pub fn precedents(&self, cell: CellKey) -> impl Iterator<Item = CellKey> + '_ {
        self.precedents.get(&cell).into_iter().flatten().copied()
    }

    /// Cells that directly reference `cell`.
    pub fn dependents(&self, cell: CellKey) -> impl Iterator<Item = CellKey> + '_ {
        self.dependents.get(&cell).into_iter().flatten().copied()
    }

    /// The full set of cells affected when `cell` changes: `cell` itself plus
    /// every cell transitively downstream of it. Cycle-safe.
    pub fn affected(&self, cell: CellKey) -> HashSet<CellKey> {
        let mut seen = HashSet::new();
        let mut queue = VecDeque::new();
        seen.insert(cell);
        queue.push_back(cell);
        while let Some(c) = queue.pop_front() {
            if let Some(deps) = self.dependents.get(&c) {
                for &d in deps {
                    if seen.insert(d) {
                        queue.push_back(d);
                    }
                }
            }
        }
        seen
    }
}

/// Collect the same-sheet cells referenced by an expression, expanding ranges.
fn refs_of(expr: &Expr) -> HashSet<CellKey> {
    let mut out = HashSet::new();
    walk(expr, &mut out);
    out
}

fn walk(expr: &Expr, out: &mut HashSet<CellKey>) {
    match expr {
        Expr::Ref(r) => {
            out.insert(key(r));
        }
        Expr::Range(range) => {
            for c in range.cells() {
                out.insert(key(&c));
            }
        }
        // Cross-sheet references and unbounded whole-column / whole-row spans
        // are not enumerated in the cell-keyed local graph. (A formula using
        // `A:A` is therefore not finely incremental — the batch evaluator stays
        // correct, but RecalcEngine may not auto-dirty it when column A grows.)
        Expr::SheetRef(_, _)
        | Expr::SheetRange(_, _)
        | Expr::ColSpan { .. }
        | Expr::RowSpan { .. } => {}
        Expr::Neg(inner) | Expr::Percent(inner) => walk(inner, out),
        Expr::Binary(_, a, b) => {
            walk(a, out);
            walk(b, out);
        }
        Expr::Func(_, args) => {
            for a in args {
                walk(a, out);
            }
        }
        Expr::Array(rows) => {
            for e in rows.iter().flatten() {
                walk(e, out);
            }
        }
        Expr::Number(_) | Expr::Text(_) | Expr::Bool(_) | Expr::Name(_) | Expr::RefError => {}
    }
}

fn key(r: &CellRef) -> CellKey {
    (r.col, r.row)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse(a1).unwrap()
    }

    fn sheet_with(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet::new("Sheet1");
        for (addr, input) in cells {
            s.set_input(cell(addr), input).unwrap();
        }
        s
    }

    fn k(a1: &str) -> CellKey {
        let r = cell(a1);
        (r.col, r.row)
    }

    #[test]
    fn records_precedents_and_dependents() {
        // C1 = A1 + B1
        let s = sheet_with(&[("A1", "1"), ("B1", "2"), ("C1", "=A1+B1")]);
        let g = DependencyGraph::build(&s);

        let mut precs: Vec<_> = g.precedents(k("C1")).collect();
        precs.sort();
        assert_eq!(precs, vec![k("A1"), k("B1")]);

        let deps: Vec<_> = g.dependents(k("A1")).collect();
        assert_eq!(deps, vec![k("C1")]);
    }

    #[test]
    fn ranges_expand_to_member_cells() {
        let s = sheet_with(&[("D1", "=SUM(A1:A3)")]);
        let g = DependencyGraph::build(&s);
        let mut precs: Vec<_> = g.precedents(k("D1")).collect();
        precs.sort();
        assert_eq!(precs, vec![k("A1"), k("A2"), k("A3")]);
    }

    #[test]
    fn affected_is_transitive() {
        // A1 -> A2 -> A3 chain.
        let s = sheet_with(&[("A1", "1"), ("A2", "=A1+1"), ("A3", "=A2+1")]);
        let g = DependencyGraph::build(&s);
        let affected = g.affected(k("A1"));
        assert!(affected.contains(&k("A1")));
        assert!(affected.contains(&k("A2")));
        assert!(affected.contains(&k("A3")));
        assert_eq!(affected.len(), 3);
    }

    #[test]
    fn affected_handles_cycles() {
        // A1 <-> A2 cycle should not loop forever.
        let mut s = Sheet::new("Sheet1");
        s.set_formula(cell("A1"), "=A2").unwrap();
        s.set_formula(cell("A2"), "=A1").unwrap();
        let g = DependencyGraph::build(&s);
        let affected = g.affected(k("A1"));
        assert_eq!(affected, [k("A1"), k("A2")].into_iter().collect());
    }

    #[test]
    fn update_cell_rewires_edges() {
        let s = sheet_with(&[("A1", "1"), ("B1", "2"), ("C1", "=A1")]);
        let mut g = DependencyGraph::build(&s);
        assert_eq!(g.dependents(k("A1")).collect::<Vec<_>>(), vec![k("C1")]);

        // Repoint C1 to reference B1 instead of A1.
        let mut s2 = s.clone();
        s2.set_formula(cell("C1"), "=B1").unwrap();
        g.update_cell(k("C1"), s2.content(2, 0));

        assert_eq!(g.dependents(k("A1")).count(), 0, "old edge dropped");
        assert_eq!(g.dependents(k("B1")).collect::<Vec<_>>(), vec![k("C1")]);
    }
}
