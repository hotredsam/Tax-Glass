//! Formula evaluation.
//!
//! Evaluation is lazy and memoized. It runs against a [`Cells`] source — either
//! a single [`Sheet`] or a whole [`Workbook`] — so the same evaluator resolves
//! both same-sheet references and sheet-qualified ones (`Sheet2!A1`). An
//! in-progress set keyed by `(sheet, col, row)` detects circular references
//! (including cross-sheet cycles) and yields `#CIRC!` instead of recursing.

use crate::formula::{BinOp, Expr};
use crate::sheet::{CellContent, Sheet};
use crate::value::{CellError, Value};
use crate::workbook::Workbook;
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// A source of cells the evaluator can read: maps sheet names to indices and
/// hands back the content stored at any `(sheet, col, row)` coordinate.
pub trait Cells {
    /// Resolve a sheet name (case-insensitive) to its index.
    fn sheet_index(&self, name: &str) -> Option<usize>;
    /// The content stored at a coordinate, if any.
    fn content(&self, sheet: usize, col: u32, row: u32) -> Option<&CellContent>;
    /// Resolve a defined name to a `(sheet index, range)`. Defaults to none.
    fn resolve_name(&self, _name: &str) -> Option<(usize, crate::address::CellRange)> {
        None
    }
    /// The `(cols, rows)` used extent of a sheet, used to bound whole-column /
    /// whole-row spans during evaluation.
    fn dimensions(&self, sheet: usize) -> (u32, u32) {
        let _ = sheet;
        (0, 0)
    }
    /// Look up a cached AI/model result for a prompt. `None` (the default) means
    /// "not cached" and surfaces as `#N/A` until a refresh populates it.
    fn ai_lookup(&self, _prompt: &str) -> Option<Value> {
        None
    }
}

impl Cells for Sheet {
    fn sheet_index(&self, name: &str) -> Option<usize> {
        if self.name.eq_ignore_ascii_case(name) {
            Some(0)
        } else {
            None
        }
    }

    fn content(&self, sheet: usize, col: u32, row: u32) -> Option<&CellContent> {
        if sheet == 0 {
            Sheet::content(self, col, row)
        } else {
            None
        }
    }

    fn dimensions(&self, sheet: usize) -> (u32, u32) {
        if sheet == 0 {
            Sheet::dimensions(self)
        } else {
            (0, 0)
        }
    }

    fn ai_lookup(&self, prompt: &str) -> Option<Value> {
        self.ai_cache().get(prompt)
    }
}

impl Cells for Workbook {
    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.index_of(name)
    }

    fn content(&self, sheet: usize, col: u32, row: u32) -> Option<&CellContent> {
        self.sheet_at(sheet).and_then(|s| s.content(col, row))
    }

    fn resolve_name(&self, name: &str) -> Option<(usize, crate::address::CellRange)> {
        self.resolve_defined_name(name)
    }

    fn dimensions(&self, sheet: usize) -> (u32, u32) {
        self.sheet_at(sheet)
            .map(|s| s.dimensions())
            .unwrap_or((0, 0))
    }
}

/// Compute the value of every populated cell in a single sheet, including
/// dynamic-array spilling: a bare-range or array-literal formula spills its grid
/// into neighboring cells, reporting `#SPILL!` if blocked.
pub fn evaluate_sheet(sheet: &Sheet) -> HashMap<(u32, u32), Value> {
    use crate::sheet::CellContent;

    let mut ev = Evaluator::new(sheet);
    let keys: Vec<(u32, u32)> = sheet.iter().map(|(k, _)| *k).collect();
    let mut out: HashMap<(u32, u32), Value> = HashMap::with_capacity(keys.len());
    // Cells filled by a spill, mapped back to the anchor that produced them.
    let mut spilled: HashMap<(u32, u32), (u32, u32)> = HashMap::new();

    // Phase 1: anchors whose top-level result is an array.
    for &(col, row) in &keys {
        let Some(CellContent::Formula { ast, .. }) = sheet.content(col, row) else {
            continue;
        };
        if !is_array_top(ast) {
            continue;
        }
        let ast = ast.clone();
        let grid = ev.eval_array(&ast);
        let rows = grid.len() as u32;
        let cols = grid.first().map(|r| r.len()).unwrap_or(0) as u32;

        // A degenerate 1×1 array just behaves as a scalar.
        if rows <= 1 && cols <= 1 {
            let v = grid
                .into_iter()
                .next()
                .and_then(|r| r.into_iter().next())
                .unwrap_or(Value::Empty);
            ev.cache.insert((0, col, row), v.clone());
            out.insert((col, row), v);
            continue;
        }

        if spill_blocked(sheet, &spilled, (col, row), rows, cols) {
            let v = Value::Error(CellError::Spill);
            ev.cache.insert((0, col, row), v.clone());
            out.insert((col, row), v);
            continue;
        }

        for (r, line) in grid.into_iter().enumerate() {
            for (c, value) in line.into_iter().enumerate() {
                let pos = (col + c as u32, row + r as u32);
                ev.cache.insert((0, pos.0, pos.1), value.clone());
                out.insert(pos, value);
                if pos != (col, row) {
                    spilled.insert(pos, (col, row));
                }
            }
        }
    }

    // Phase 2: everything else (scalars + literals), reusing the seeded cache so
    // formulas that reference spilled cells see their values.
    for &(col, row) in &keys {
        out.entry((col, row))
            .or_insert_with(|| ev.value_at(0, col, row));
    }

    out
}

/// Whether a formula's top-level result spills (bare range or array literal).
fn is_array_top(ast: &Expr) -> bool {
    matches!(
        ast,
        Expr::Range(_) | Expr::SheetRange(_, _) | Expr::Array(_)
    )
}

/// True if a spill region (other than its anchor) would overwrite a stored cell
/// or a region already claimed by another spill.
fn spill_blocked(
    sheet: &Sheet,
    spilled: &HashMap<(u32, u32), (u32, u32)>,
    anchor: (u32, u32),
    rows: u32,
    cols: u32,
) -> bool {
    for r in 0..rows {
        for c in 0..cols {
            let pos = (anchor.0 + c, anchor.1 + r);
            if pos == anchor {
                continue;
            }
            if sheet.content(pos.0, pos.1).is_some() || spilled.contains_key(&pos) {
                return true;
            }
        }
    }
    false
}

/// Incrementally evaluate a set of target cells on a single sheet, seeding the
/// evaluator's cache with already-known *clean* values so untouched precedents
/// are reused instead of recomputed. Returns the freshly computed value for
/// each target. Used by [`crate::recalc::RecalcEngine`].
pub fn evaluate_targets(
    sheet: &Sheet,
    clean: &HashMap<(u32, u32), Value>,
    targets: &HashSet<(u32, u32)>,
) -> HashMap<(u32, u32), Value> {
    let mut ev = Evaluator::new(sheet);
    ev.clean_seed = Some(clean);
    let mut out = HashMap::with_capacity(targets.len());
    for &(col, row) in targets {
        out.insert((col, row), ev.value_at(0, col, row));
    }
    out
}

/// Evaluate a single sheet in **iterative mode**, allowing intentional circular
/// references to converge. Formula cells start at 0 and are recomputed from the
/// previous iteration's snapshot until the largest numeric change drops below
/// `epsilon` or `max_iterations` is reached.
pub fn evaluate_sheet_iterative(
    sheet: &Sheet,
    max_iterations: u32,
    epsilon: f64,
) -> HashMap<(u32, u32), Value> {
    use crate::sheet::CellContent;

    // Seed: literals at their value, formula cells at 0.
    let mut snapshot: HashMap<(usize, u32, u32), Value> = HashMap::new();
    let mut formula_keys: Vec<(u32, u32)> = Vec::new();
    for (&(col, row), content) in sheet.iter() {
        match content {
            CellContent::Literal(v) => {
                snapshot.insert((0, col, row), v.clone());
            }
            CellContent::Formula { .. } => {
                snapshot.insert((0, col, row), Value::Number(0.0));
                formula_keys.push((col, row));
            }
        }
    }

    for _ in 0..max_iterations.max(1) {
        let mut next = snapshot.clone();
        let mut max_delta = 0.0_f64;
        for &(col, row) in &formula_keys {
            let new_val = match sheet.content(col, row) {
                Some(CellContent::Formula { ast, .. }) => {
                    let ast = ast.clone();
                    let mut ev = Evaluator::with_snapshot(sheet, 0, &snapshot);
                    ev.eval(&ast)
                }
                _ => Value::Empty,
            };
            let old = snapshot.get(&(0, col, row));
            max_delta = max_delta.max(numeric_delta(old, &new_val));
            next.insert((0, col, row), new_val);
        }
        snapshot = next;
        if max_delta < epsilon {
            break;
        }
    }

    snapshot
        .into_iter()
        .map(|((_, col, row), v)| ((col, row), v))
        .collect()
}

/// Magnitude of change between two values for convergence testing. Numeric
/// changes use the absolute difference; any non-numeric change (or appearance)
/// counts as infinite so iteration continues.
fn numeric_delta(old: Option<&Value>, new: &Value) -> f64 {
    match (old, new) {
        (Some(Value::Number(a)), Value::Number(b)) => (a - b).abs(),
        (Some(o), n) if o == n => 0.0,
        _ => f64::INFINITY,
    }
}

/// Compute every sheet of a workbook, resolving cross-sheet references. Returns
/// each sheet's grid keyed by sheet name.
pub fn evaluate_workbook(wb: &Workbook) -> HashMap<String, HashMap<(u32, u32), Value>> {
    let mut ev = Evaluator::new(wb);
    let mut out = HashMap::with_capacity(wb.len());
    for (idx, sheet) in wb.sheets().iter().enumerate() {
        let mut grid = HashMap::with_capacity(sheet.len());
        for (&(col, row), _) in sheet.iter() {
            grid.insert((col, row), ev.value_at(idx, col, row));
        }
        out.insert(sheet.name.clone(), grid);
    }
    out
}

struct Evaluator<'a> {
    cells: &'a dyn Cells,
    /// Sheet index whose scope unqualified references resolve against.
    current: usize,
    cache: HashMap<(usize, u32, u32), Value>,
    in_progress: HashSet<(usize, u32, u32)>,
    /// In iterative mode, references read from this fixed snapshot of the
    /// previous iteration instead of recursing — so circular formulas advance
    /// one step per iteration rather than tripping the cycle guard.
    snapshot: Option<&'a HashMap<(usize, u32, u32), Value>>,
    /// Already-computed *clean* values for sheet 0, borrowed (not copied) so an
    /// incremental recalc reuses them in O(1) without seeding the cache.
    clean_seed: Option<&'a HashMap<(u32, u32), Value>>,
}

impl<'a> Evaluator<'a> {
    fn new(cells: &'a dyn Cells) -> Self {
        Evaluator {
            cells,
            current: 0,
            cache: HashMap::new(),
            in_progress: HashSet::new(),
            snapshot: None,
            clean_seed: None,
        }
    }

    fn with_snapshot(
        cells: &'a dyn Cells,
        current: usize,
        snapshot: &'a HashMap<(usize, u32, u32), Value>,
    ) -> Self {
        Evaluator {
            cells,
            current,
            cache: HashMap::new(),
            in_progress: HashSet::new(),
            snapshot: Some(snapshot),
            clean_seed: None,
        }
    }

    /// The computed value at a coordinate on a specific sheet, memoized and
    /// cycle-guarded. Evaluating a formula switches the "current" sheet to the
    /// cell's own sheet so its unqualified refs resolve locally.
    fn value_at(&mut self, sheet: usize, col: u32, row: u32) -> Value {
        let key = (sheet, col, row);
        // Iterative mode: read the previous iteration's value, no recursion.
        if let Some(snap) = self.snapshot {
            return snap.get(&key).cloned().unwrap_or(Value::Empty);
        }
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        // Reuse already-computed clean values (incremental recalc) without
        // copying them into the cache up front.
        if sheet == 0 {
            if let Some(seed) = self.clean_seed {
                if let Some(v) = seed.get(&(col, row)) {
                    return v.clone();
                }
            }
        }
        if self.in_progress.contains(&key) {
            return Value::Error(CellError::Circular);
        }

        let value = match self.cells.content(sheet, col, row) {
            None => Value::Empty,
            Some(CellContent::Literal(v)) => v.clone(),
            Some(CellContent::Formula { ast, .. }) => {
                let ast = ast.clone();
                let saved = self.current;
                self.current = sheet;
                self.in_progress.insert(key);
                let v = self.eval(&ast);
                self.in_progress.remove(&key);
                self.current = saved;
                v
            }
        };
        self.cache.insert(key, value.clone());
        value
    }

    /// Resolve a sheet name to its index, or `None` (→ `#REF!`).
    fn sheet_index(&self, name: &str) -> Option<usize> {
        self.cells.sheet_index(name)
    }

    /// Evaluate an expression to a scalar value.
    fn eval(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(t) => Value::Text(t.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Ref(r) => self.value_at(self.current, r.col, r.row),
            Expr::SheetRef(name, r) => match self.sheet_index(name) {
                Some(idx) => self.value_at(idx, r.col, r.row),
                None => Value::Error(CellError::Ref),
            },
            // A bare range/span used as a scalar has no implicit intersection.
            Expr::Range(_)
            | Expr::SheetRange(_, _)
            | Expr::ColSpan { .. }
            | Expr::RowSpan { .. } => Value::Error(CellError::Value),
            Expr::Name(name) => match self.cells.resolve_name(name) {
                // A 1×1 named range resolves to that cell's value; a larger one
                // is a range and has no scalar meaning here.
                Some((idx, range)) if range.len() == 1 => {
                    self.value_at(idx, range.start.col, range.start.row)
                }
                Some(_) => Value::Error(CellError::Value),
                None => Value::Error(CellError::Name),
            },
            Expr::RefError => Value::Error(CellError::Ref),
            // An array in a scalar slot collapses to its top-left element.
            Expr::Array(rows) => match rows.first().and_then(|r| r.first()) {
                Some(e) => self.eval(e),
                None => Value::Empty,
            },
            Expr::Neg(inner) => match self.eval(inner).as_number() {
                Ok(n) => Value::Number(-n),
                Err(e) => Value::Error(e),
            },
            Expr::Percent(inner) => match self.eval(inner).as_number() {
                Ok(n) => Value::Number(n / 100.0),
                Err(e) => Value::Error(e),
            },
            Expr::Binary(op, lhs, rhs) => self.eval_binary(*op, lhs, rhs),
            Expr::Func(name, args) => self.eval_func(name, args),
        }
    }

    fn eval_binary(&mut self, op: BinOp, lhs: &Expr, rhs: &Expr) -> Value {
        let l = self.eval(lhs);
        let r = self.eval(rhs);
        if let Value::Error(e) = l {
            return Value::Error(e);
        }
        if let Value::Error(e) = r {
            return Value::Error(e);
        }

        match op {
            BinOp::Concat => Value::Text(format!("{}{}", l.as_text(), r.as_text())),
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => {
                self.compare(op, &l, &r)
            }
            BinOp::Add | BinOp::Sub | BinOp::Mul | BinOp::Div | BinOp::Pow => {
                let (a, b) = match (l.as_number(), r.as_number()) {
                    (Ok(a), Ok(b)) => (a, b),
                    (Err(e), _) | (_, Err(e)) => return Value::Error(e),
                };
                match op {
                    BinOp::Add => Value::Number(a + b),
                    BinOp::Sub => Value::Number(a - b),
                    BinOp::Mul => Value::Number(a * b),
                    BinOp::Div => {
                        if b == 0.0 {
                            Value::Error(CellError::Div0)
                        } else {
                            Value::Number(a / b)
                        }
                    }
                    BinOp::Pow => {
                        let p = a.powf(b);
                        if p.is_nan() || p.is_infinite() {
                            Value::Error(CellError::Num)
                        } else {
                            Value::Number(p)
                        }
                    }
                    _ => unreachable!(),
                }
            }
        }
    }

    fn compare(&self, op: BinOp, l: &Value, r: &Value) -> Value {
        let ord = compare_values(l, r);
        let result = match op {
            BinOp::Eq => ord == Ordering::Equal,
            BinOp::Ne => ord != Ordering::Equal,
            BinOp::Lt => ord == Ordering::Less,
            BinOp::Gt => ord == Ordering::Greater,
            BinOp::Le => ord != Ordering::Greater,
            BinOp::Ge => ord != Ordering::Less,
            _ => unreachable!(),
        };
        Value::Bool(result)
    }

    /// Flatten a function argument into a list of scalar values, expanding
    /// ranges (same-sheet or sheet-qualified) cell-by-cell.
    fn flatten(&mut self, expr: &Expr) -> Vec<Value> {
        match expr {
            Expr::Range(range) => {
                let sheet = self.current;
                range
                    .cells()
                    .map(|c| self.value_at(sheet, c.col, c.row))
                    .collect()
            }
            Expr::SheetRange(name, range) => match self.sheet_index(name) {
                Some(idx) => range
                    .cells()
                    .map(|c| self.value_at(idx, c.col, c.row))
                    .collect(),
                None => vec![Value::Error(CellError::Ref)],
            },
            // Whole-column / whole-row spans expand over the sheet's used extent
            // (blanks contribute nothing to numeric aggregates, just as in Excel).
            Expr::ColSpan { start, end } => {
                let (_, rows) = self.cells.dimensions(self.current);
                let sheet = self.current;
                let mut out = Vec::new();
                for col in *start..=*end {
                    for row in 0..rows {
                        out.push(self.value_at(sheet, col, row));
                    }
                }
                out
            }
            Expr::RowSpan { start, end } => {
                let (cols, _) = self.cells.dimensions(self.current);
                let sheet = self.current;
                let mut out = Vec::new();
                for row in *start..=*end {
                    for col in 0..cols {
                        out.push(self.value_at(sheet, col, row));
                    }
                }
                out
            }
            // A named range expands to its target cells when used as an argument.
            Expr::Name(name) => match self.cells.resolve_name(name) {
                Some((idx, range)) => range
                    .cells()
                    .map(|c| self.value_at(idx, c.col, c.row))
                    .collect(),
                None => vec![Value::Error(CellError::Name)],
            },
            Expr::Array(rows) => rows.iter().flatten().map(|e| self.eval(e)).collect(),
            other => vec![self.eval(other)],
        }
    }

    /// Evaluate an expression to a 2-D array (rows × cols) for spilling. Array
    /// literals and bounded ranges produce real grids; anything else is a 1×1.
    fn eval_array(&mut self, expr: &Expr) -> Vec<Vec<Value>> {
        match expr {
            Expr::Array(rows) => rows
                .iter()
                .map(|r| r.iter().map(|e| self.eval(e)).collect())
                .collect(),
            Expr::Range(range) => self.range_array(self.current, range),
            Expr::SheetRange(name, range) => match self.sheet_index(name) {
                Some(idx) => self.range_array(idx, range),
                None => vec![vec![Value::Error(CellError::Ref)]],
            },
            other => vec![vec![self.eval(other)]],
        }
    }

    fn range_array(&mut self, sheet: usize, range: &crate::address::CellRange) -> Vec<Vec<Value>> {
        (range.start.row..=range.end.row)
            .map(|row| {
                (range.start.col..=range.end.col)
                    .map(|col| self.value_at(sheet, col, row))
                    .collect()
            })
            .collect()
    }

    /// Collect the numeric values from arguments, skipping blanks and
    /// non-numeric text (as Excel does within ranges), but propagating errors.
    fn collect_numbers(&mut self, args: &[Expr]) -> Result<Vec<f64>, CellError> {
        let mut nums = Vec::new();
        for arg in args {
            for v in self.flatten(arg) {
                match v {
                    Value::Error(e) => return Err(e),
                    Value::Empty => {}
                    Value::Number(n) => nums.push(n),
                    Value::Bool(b) => nums.push(if b { 1.0 } else { 0.0 }),
                    Value::Text(t) => {
                        if let Ok(n) = t.trim().parse::<f64>() {
                            nums.push(n);
                        }
                    }
                }
            }
        }
        Ok(nums)
    }

    fn eval_func(&mut self, name: &str, args: &[Expr]) -> Value {
        match name {
            // --- aggregation ---
            "SUM" => self.numeric_agg(args, 0.0, |acc, n| acc + n),
            "PRODUCT" => self.numeric_agg(args, 1.0, |acc, n| acc * n),
            "MIN" => self.minmax(args, true),
            "MAX" => self.minmax(args, false),
            "AVERAGE" => match self.collect_numbers(args) {
                Err(e) => Value::Error(e),
                Ok(ns) if ns.is_empty() => Value::Error(CellError::Div0),
                Ok(ns) => Value::Number(ns.iter().sum::<f64>() / ns.len() as f64),
            },
            "COUNT" => match self.collect_numbers(args) {
                Err(e) => Value::Error(e),
                Ok(ns) => Value::Number(ns.len() as f64),
            },
            "SUMIF" => self.func_sumif(args),
            "AVERAGEIF" => self.func_averageif(args),
            "COUNTIF" => self.func_countif(args),
            "SUMIFS" => self.func_ifs(args, IfsKind::Sum),
            "AVERAGEIFS" => self.func_ifs(args, IfsKind::Average),
            "COUNTIFS" => self.func_ifs(args, IfsKind::Count),
            "COUNTA" => {
                let mut count = 0;
                for arg in args {
                    for v in self.flatten(arg) {
                        if let Value::Error(e) = v {
                            return Value::Error(e);
                        }
                        if v != Value::Empty {
                            count += 1;
                        }
                    }
                }
                Value::Number(count as f64)
            }

            // --- math (scalar) ---
            "ABS" => self.unary_math(args, f64::abs),
            "SQRT" => self.scalar1(args, |n| {
                if n < 0.0 {
                    Value::Error(CellError::Num)
                } else {
                    Value::Number(n.sqrt())
                }
            }),
            "INT" => self.unary_math(args, f64::floor),
            "TRUNC" => self.unary_math(args, f64::trunc),
            "SIGN" => self.unary_math(args, f64::signum),
            "POWER" => self.scalar2(args, |a, b| {
                let p = a.powf(b);
                if p.is_finite() {
                    Value::Number(p)
                } else {
                    Value::Error(CellError::Num)
                }
            }),
            "MOD" => self.scalar2(args, |a, b| {
                if b == 0.0 {
                    Value::Error(CellError::Div0)
                } else {
                    Value::Number(a - b * (a / b).floor())
                }
            }),
            // --- trigonometry ---
            "PI" => {
                if args.is_empty() {
                    Value::Number(std::f64::consts::PI)
                } else {
                    Value::Error(CellError::Value)
                }
            }
            "SIN" => self.scalar1(args, |n| Value::Number(n.sin())),
            "COS" => self.scalar1(args, |n| Value::Number(n.cos())),
            "TAN" => self.scalar1(args, |n| Value::Number(n.tan())),
            "ASIN" => self.scalar1(args, |n| domain_num(n.abs() <= 1.0, n.asin())),
            "ACOS" => self.scalar1(args, |n| domain_num(n.abs() <= 1.0, n.acos())),
            "ATAN" => self.scalar1(args, |n| Value::Number(n.atan())),
            // Excel's ATAN2 takes (x, y) and returns atan(y/x).
            "ATAN2" => self.scalar2(args, |x, y| Value::Number(y.atan2(x))),
            "DEGREES" => self.scalar1(args, |n| Value::Number(n.to_degrees())),
            "RADIANS" => self.scalar1(args, |n| Value::Number(n.to_radians())),
            // --- hyperbolic ---
            "SINH" => self.scalar1(args, |n| Value::Number(n.sinh())),
            "COSH" => self.scalar1(args, |n| Value::Number(n.cosh())),
            "TANH" => self.scalar1(args, |n| Value::Number(n.tanh())),
            "ASINH" => self.scalar1(args, |n| Value::Number(n.asinh())),
            "ACOSH" => self.scalar1(args, |n| domain_num(n >= 1.0, n.acosh())),
            "ATANH" => self.scalar1(args, |n| domain_num(n.abs() < 1.0, n.atanh())),
            // --- logarithms & exponentials ---
            "LN" => self.scalar1(args, |n| domain_num(n > 0.0, n.ln())),
            "LOG10" => self.scalar1(args, |n| domain_num(n > 0.0, n.log10())),
            "EXP" => self.scalar1(args, |n| Value::Number(n.exp())),
            "SQRTPI" => self.scalar1(args, |n| {
                domain_num(n >= 0.0, (n * std::f64::consts::PI).sqrt())
            }),
            "LOG" => self.func_log(args),
            "ROUND" => self.round_family(args, RoundMode::Half),
            "ROUNDUP" => self.round_family(args, RoundMode::Up),
            "ROUNDDOWN" => self.round_family(args, RoundMode::Down),
            // --- random ---
            "RAND" => {
                if args.is_empty() {
                    Value::Number(next_rand())
                } else {
                    Value::Error(CellError::Value)
                }
            }
            "RANDBETWEEN" => self.scalar2(args, |lo, hi| {
                let lo = lo.ceil();
                let hi = hi.floor();
                if hi < lo {
                    Value::Error(CellError::Num)
                } else {
                    let span = hi - lo + 1.0;
                    Value::Number(lo + (next_rand() * span).floor())
                }
            }),
            // --- central tendency / dispersion ---
            "MEDIAN" => self.stat_median(args),
            "MODE" | "MODE.SNGL" => self.stat_mode(args),
            "GEOMEAN" => self.stat_geomean(args),
            "HARMEAN" => self.stat_harmean(args),
            "TRIMMEAN" => self.func_trimmean(args),
            // --- integer / combinatorial math ---
            "GCD" => self.func_gcd(args),
            "LCM" => self.func_lcm(args),
            "FACT" => self.scalar1(args, fact_value),
            "FACTDOUBLE" => self.scalar1(args, factdouble_value),
            "COMBIN" => self.scalar2(args, |n, k| combin_value(n, k, false)),
            "COMBINA" => self.scalar2(args, |n, k| combin_value(n, k, true)),
            "PERMUT" => self.scalar2(args, permut_value),
            "PERMUTATIONA" => self.scalar2(args, |n, k| Value::Number(n.trunc().powf(k.trunc()))),
            "QUOTIENT" => self.scalar2(args, |a, b| {
                if b == 0.0 {
                    Value::Error(CellError::Div0)
                } else {
                    Value::Number((a / b).trunc())
                }
            }),
            "ROMAN" => self.scalar1(args, roman_value),
            "ARABIC" => self.scalar_text1(args, arabic_value),
            "BASE" => self.func_base(args),
            "DECIMAL" => self.func_decimal(args),
            "MROUND" => self.scalar2(args, |n, m| {
                if m == 0.0 {
                    Value::Number(0.0)
                } else {
                    Value::Number((n / m).round() * m)
                }
            }),
            "CEILING" | "CEILING.MATH" => self.ceiling_floor(args, true),
            "FLOOR" | "FLOOR.MATH" => self.ceiling_floor(args, false),
            "EVEN" => self.scalar1(args, |n| Value::Number(round_to_parity(n, true))),
            "ODD" => self.scalar1(args, |n| Value::Number(round_to_parity(n, false))),
            "SUMPRODUCT" => self.func_sumproduct(args),
            "SUBTOTAL" => self.func_subtotal(args),
            "AGGREGATE" => self.func_aggregate(args),

            // --- logical ---
            "IF" => self.func_if(args),
            "AND" => self.bool_agg(args, true),
            "OR" => self.bool_agg(args, false),
            "NOT" => {
                if args.len() != 1 {
                    return Value::Error(CellError::Value);
                }
                match self.eval(&args[0]).as_bool() {
                    Ok(b) => Value::Bool(!b),
                    Err(e) => Value::Error(e),
                }
            }
            "TRUE" => Value::Bool(true),
            "FALSE" => Value::Bool(false),

            // --- text ---
            "CONCAT" | "CONCATENATE" => {
                let mut s = String::new();
                for arg in args {
                    for v in self.flatten(arg) {
                        if let Value::Error(e) = v {
                            return Value::Error(e);
                        }
                        s.push_str(&v.as_text());
                    }
                }
                Value::Text(s)
            }
            "LEN" => self.scalar_text1(args, |s| Value::Number(s.chars().count() as f64)),
            "UPPER" => self.scalar_text1(args, |s| Value::Text(s.to_uppercase())),
            "LOWER" => self.scalar_text1(args, |s| Value::Text(s.to_lowercase())),
            "TRIM" => self.scalar_text1(args, |s| Value::Text(s.trim().to_string())),

            // --- GlassSheet extensions (not in Excel) ---
            // COMPARE(a, b): -1 if a<b, 0 if equal, 1 if a>b (Excel has no
            // single ordering function).
            "COMPARE" => {
                if args.len() != 2 {
                    return Value::Error(CellError::Value);
                }
                let a = self.eval(&args[0]);
                let b = self.eval(&args[1]);
                if let Value::Error(e) = a {
                    return Value::Error(e);
                }
                if let Value::Error(e) = b {
                    return Value::Error(e);
                }
                Value::Number(match compare_values(&a, &b) {
                    Ordering::Less => -1.0,
                    Ordering::Equal => 0.0,
                    Ordering::Greater => 1.0,
                })
            }
            // SIMILARITY(text1, text2): 0..1 fuzzy match (1 - normalized edit
            // distance). Handy for de-duping and reconciliation.
            "SIMILARITY" => {
                if args.len() != 2 {
                    return Value::Error(CellError::Value);
                }
                let a = self.eval(&args[0]);
                let b = self.eval(&args[1]);
                if let Value::Error(e) = a {
                    return Value::Error(e);
                }
                if let Value::Error(e) = b {
                    return Value::Error(e);
                }
                Value::Number(similarity(&a.as_text(), &b.as_text()))
            }
            // AI(prompt, [hint]): result of an AI/model call, served from the
            // workbook's refreshable cache. Returns #N/A while a result is
            // pending a refresh (see crate::ai).
            "AI" => {
                if args.is_empty() || args.len() > 2 {
                    return Value::Error(CellError::Value);
                }
                let prompt = self.eval(&args[0]);
                if let Value::Error(e) = prompt {
                    return Value::Error(e);
                }
                match self.cells.ai_lookup(&prompt.as_text()) {
                    Some(v) => v,
                    None => Value::Error(CellError::NA),
                }
            }

            _ => Value::Error(CellError::Name),
        }
    }

    // --- conditional aggregation ---

    /// `SUMIF(range, criteria, [sum_range])`.
    fn func_sumif(&mut self, args: &[Expr]) -> Value {
        if args.len() < 2 || args.len() > 3 {
            return Value::Error(CellError::Value);
        }
        let test = self.flatten(&args[0]);
        let criteria = self.eval(&args[1]);
        let sum_vals = if args.len() == 3 {
            self.flatten(&args[2])
        } else {
            test.clone()
        };
        let mut total = 0.0;
        for (i, t) in test.iter().enumerate() {
            if let Value::Error(e) = t {
                return Value::Error(*e);
            }
            if criteria_matches(t, &criteria) {
                if let Some(Ok(n)) = sum_vals.get(i).map(|v| v.as_number()) {
                    total += n;
                }
            }
        }
        Value::Number(total)
    }

    /// `COUNTIF(range, criteria)`.
    fn func_countif(&mut self, args: &[Expr]) -> Value {
        if args.len() != 2 {
            return Value::Error(CellError::Value);
        }
        let test = self.flatten(&args[0]);
        let criteria = self.eval(&args[1]);
        let count = test
            .iter()
            .filter(|t| criteria_matches(t, &criteria))
            .count();
        Value::Number(count as f64)
    }

    /// `AVERAGEIF(range, criteria, [avg_range])`.
    fn func_averageif(&mut self, args: &[Expr]) -> Value {
        if args.len() < 2 || args.len() > 3 {
            return Value::Error(CellError::Value);
        }
        let test = self.flatten(&args[0]);
        let criteria = self.eval(&args[1]);
        let avg_vals = if args.len() == 3 {
            self.flatten(&args[2])
        } else {
            test.clone()
        };
        let mut sum = 0.0;
        let mut count = 0u32;
        for (i, t) in test.iter().enumerate() {
            if criteria_matches(t, &criteria) {
                if let Some(Ok(n)) = avg_vals.get(i).map(|v| v.as_number()) {
                    sum += n;
                    count += 1;
                }
            }
        }
        if count == 0 {
            Value::Error(CellError::Div0)
        } else {
            Value::Number(sum / count as f64)
        }
    }

    /// The `*IFS` family: `SUMIFS(sum_range, crit_range, crit, ...)`,
    /// `COUNTIFS(crit_range, crit, ...)`, `AVERAGEIFS(avg_range, ...)`.
    fn func_ifs(&mut self, args: &[Expr], kind: IfsKind) -> Value {
        // COUNTIFS has only (range, criteria) pairs; the others lead with the
        // aggregation range.
        let (agg, pairs): (Option<Vec<Value>>, &[Expr]) = match kind {
            IfsKind::Count => (None, args),
            _ => {
                if args.is_empty() {
                    return Value::Error(CellError::Value);
                }
                (Some(self.flatten(&args[0])), &args[1..])
            }
        };
        if pairs.is_empty() || pairs.len() % 2 != 0 {
            return Value::Error(CellError::Value);
        }

        // Evaluate each (range, criteria) pair.
        let mut tests: Vec<(Vec<Value>, Value)> = Vec::new();
        let mut len = agg.as_ref().map(|a| a.len());
        let mut i = 0;
        while i < pairs.len() {
            let range = self.flatten(&pairs[i]);
            let criteria = self.eval(&pairs[i + 1]);
            len = Some(len.unwrap_or(range.len()).min(range.len()));
            tests.push((range, criteria));
            i += 2;
        }
        let len = len.unwrap_or(0);

        let mut sum = 0.0;
        let mut count = 0u32;
        for idx in 0..len {
            let all = tests
                .iter()
                .all(|(range, crit)| range.get(idx).is_some_and(|v| criteria_matches(v, crit)));
            if !all {
                continue;
            }
            count += 1;
            if let Some(agg) = &agg {
                if let Some(Ok(n)) = agg.get(idx).map(|v| v.as_number()) {
                    sum += n;
                }
            }
        }

        match kind {
            IfsKind::Count => Value::Number(count as f64),
            IfsKind::Sum => Value::Number(sum),
            IfsKind::Average => {
                if count == 0 {
                    Value::Error(CellError::Div0)
                } else {
                    Value::Number(sum / count as f64)
                }
            }
        }
    }

    fn sorted_numbers(&mut self, args: &[Expr]) -> Result<Vec<f64>, CellError> {
        let mut ns = self.collect_numbers(args)?;
        ns.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        Ok(ns)
    }

    fn stat_median(&mut self, args: &[Expr]) -> Value {
        match self.sorted_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) if ns.is_empty() => Value::Error(CellError::Num),
            Ok(ns) => {
                let mid = ns.len() / 2;
                let m = if ns.len() % 2 == 0 {
                    (ns[mid - 1] + ns[mid]) / 2.0
                } else {
                    ns[mid]
                };
                Value::Number(m)
            }
        }
    }

    fn stat_mode(&mut self, args: &[Expr]) -> Value {
        match self.collect_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) => {
                let mut best: Option<(f64, usize)> = None;
                for (i, &n) in ns.iter().enumerate() {
                    let count = ns[i..].iter().filter(|&&x| x == n).count()
                        + ns[..i].iter().filter(|&&x| x == n).count();
                    if count > 1 && best.map(|(_, c)| count > c).unwrap_or(true) {
                        best = Some((n, count));
                    }
                }
                match best {
                    Some((n, _)) => Value::Number(n),
                    None => Value::Error(CellError::NA),
                }
            }
        }
    }

    fn stat_geomean(&mut self, args: &[Expr]) -> Value {
        match self.collect_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) if ns.is_empty() || ns.iter().any(|n| *n <= 0.0) => Value::Error(CellError::Num),
            Ok(ns) => {
                let sum_ln: f64 = ns.iter().map(|n| n.ln()).sum();
                Value::Number((sum_ln / ns.len() as f64).exp())
            }
        }
    }

    fn stat_harmean(&mut self, args: &[Expr]) -> Value {
        match self.collect_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) if ns.is_empty() || ns.iter().any(|n| *n <= 0.0) => Value::Error(CellError::Num),
            Ok(ns) => {
                let sum_recip: f64 = ns.iter().map(|n| 1.0 / n).sum();
                Value::Number(ns.len() as f64 / sum_recip)
            }
        }
    }

    /// TRIMMEAN(data, percent): mean after trimming `percent` of values split
    /// evenly between the high and low ends.
    fn func_trimmean(&mut self, args: &[Expr]) -> Value {
        if args.len() != 2 {
            return Value::Error(CellError::Value);
        }
        let percent = match self.eval(&args[1]).as_number() {
            Ok(p) if (0.0..1.0).contains(&p) => p,
            _ => return Value::Error(CellError::Num),
        };
        let ns = match self.sorted_numbers(&args[..1]) {
            Ok(ns) if !ns.is_empty() => ns,
            Ok(_) => return Value::Error(CellError::Num),
            Err(e) => return Value::Error(e),
        };
        // Trim an even count from each end.
        let trim = ((ns.len() as f64 * percent) / 2.0).floor() as usize;
        let slice = &ns[trim..ns.len() - trim];
        if slice.is_empty() {
            return Value::Error(CellError::Num);
        }
        Value::Number(slice.iter().sum::<f64>() / slice.len() as f64)
    }

    fn func_gcd(&mut self, args: &[Expr]) -> Value {
        match self.collect_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) if ns.is_empty() || ns.iter().any(|n| *n < 0.0) => Value::Error(CellError::Num),
            Ok(ns) => {
                let g = ns.iter().fold(0u64, |acc, n| gcd(acc, n.trunc() as u64));
                Value::Number(g as f64)
            }
        }
    }

    fn func_lcm(&mut self, args: &[Expr]) -> Value {
        match self.collect_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) if ns.is_empty() || ns.iter().any(|n| *n < 0.0) => Value::Error(CellError::Num),
            Ok(ns) => {
                let mut l = 1u64;
                for n in ns {
                    let v = n.trunc() as u64;
                    if v == 0 {
                        return Value::Number(0.0);
                    }
                    l = l / gcd(l, v) * v;
                }
                Value::Number(l as f64)
            }
        }
    }

    /// BASE(number, radix, [min_length]) → string in the given radix.
    fn func_base(&mut self, args: &[Expr]) -> Value {
        if args.len() < 2 || args.len() > 3 {
            return Value::Error(CellError::Value);
        }
        let n = match self.eval(&args[0]).as_number() {
            Ok(n) if n >= 0.0 => n.trunc() as u64,
            _ => return Value::Error(CellError::Num),
        };
        let radix = match self.eval(&args[1]).as_number() {
            Ok(r) if (2.0..=36.0).contains(&r) => r as u32,
            _ => return Value::Error(CellError::Num),
        };
        let min_len = if args.len() == 3 {
            self.eval(&args[2]).as_number().unwrap_or(0.0).max(0.0) as usize
        } else {
            0
        };
        let mut s = to_radix(n, radix);
        while s.len() < min_len {
            s.insert(0, '0');
        }
        Value::Text(s)
    }

    /// DECIMAL(text, radix) → number parsed from the given radix.
    fn func_decimal(&mut self, args: &[Expr]) -> Value {
        if args.len() != 2 {
            return Value::Error(CellError::Value);
        }
        let text = self.eval(&args[0]).as_text();
        let radix = match self.eval(&args[1]).as_number() {
            Ok(r) if (2.0..=36.0).contains(&r) => r as u32,
            _ => return Value::Error(CellError::Num),
        };
        match u64::from_str_radix(text.trim(), radix) {
            Ok(n) => Value::Number(n as f64),
            Err(_) => Value::Error(CellError::Num),
        }
    }

    /// LOG(number, [base=10]).
    fn func_log(&mut self, args: &[Expr]) -> Value {
        if args.is_empty() || args.len() > 2 {
            return Value::Error(CellError::Value);
        }
        let n = match self.eval(&args[0]).as_number() {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        let base = if args.len() == 2 {
            match self.eval(&args[1]).as_number() {
                Ok(b) => b,
                Err(e) => return Value::Error(e),
            }
        } else {
            10.0
        };
        if n <= 0.0 || base <= 0.0 || base == 1.0 {
            Value::Error(CellError::Num)
        } else {
            Value::Number(n.log(base))
        }
    }

    /// CEILING/FLOOR with optional significance (default 1).
    fn ceiling_floor(&mut self, args: &[Expr], up: bool) -> Value {
        if args.is_empty() || args.len() > 2 {
            return Value::Error(CellError::Value);
        }
        let n = match self.eval(&args[0]).as_number() {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        let sig = if args.len() == 2 {
            match self.eval(&args[1]).as_number() {
                Ok(s) => s,
                Err(e) => return Value::Error(e),
            }
        } else {
            1.0
        };
        if sig == 0.0 {
            return Value::Number(0.0);
        }
        let q = n / sig;
        let rounded = if up { q.ceil() } else { q.floor() };
        Value::Number(rounded * sig)
    }

    /// SUMPRODUCT: element-wise product of equal-length arrays, summed.
    fn func_sumproduct(&mut self, args: &[Expr]) -> Value {
        if args.is_empty() {
            return Value::Error(CellError::Value);
        }
        let arrays: Vec<Vec<f64>> = args
            .iter()
            .map(|a| {
                self.flatten(a)
                    .iter()
                    .map(|v| v.as_number().unwrap_or(0.0))
                    .collect()
            })
            .collect();
        let len = arrays[0].len();
        if arrays.iter().any(|a| a.len() != len) {
            return Value::Error(CellError::Value);
        }
        let mut total = 0.0;
        for i in 0..len {
            let mut product = 1.0;
            for a in &arrays {
                product *= a[i];
            }
            total += product;
        }
        Value::Number(total)
    }

    /// SUBTOTAL(func_num, ref...). Supports the common function numbers (and
    /// their 1xx "ignore hidden" equivalents, treated identically here).
    fn func_subtotal(&mut self, args: &[Expr]) -> Value {
        if args.len() < 2 {
            return Value::Error(CellError::Value);
        }
        let func = match self.eval(&args[0]).as_number() {
            Ok(n) => (n as i64) % 100,
            Err(e) => return Value::Error(e),
        };
        self.aggregate_by_num(func, &args[1..], false)
    }

    /// AGGREGATE(func_num, options, ref...). Options 2/3/6/7 ignore error values.
    fn func_aggregate(&mut self, args: &[Expr]) -> Value {
        if args.len() < 3 {
            return Value::Error(CellError::Value);
        }
        let func = match self.eval(&args[0]).as_number() {
            Ok(n) => n as i64,
            Err(e) => return Value::Error(e),
        };
        let option = match self.eval(&args[1]).as_number() {
            Ok(n) => n as i64,
            Err(e) => return Value::Error(e),
        };
        let skip_errors = matches!(option, 2 | 3 | 6 | 7);
        self.aggregate_by_num(func, &args[2..], skip_errors)
    }

    /// Shared dispatch for SUBTOTAL/AGGREGATE function numbers.
    fn aggregate_by_num(&mut self, func: i64, refs: &[Expr], skip_errors: bool) -> Value {
        if func == 3 {
            // COUNTA: count non-empty.
            let mut count = 0;
            for arg in refs {
                for v in self.flatten(arg) {
                    if let Value::Error(e) = v {
                        if !skip_errors {
                            return Value::Error(e);
                        }
                    } else if v != Value::Empty {
                        count += 1;
                    }
                }
            }
            return Value::Number(count as f64);
        }
        let nums = match self.gather_numbers(refs, skip_errors) {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        match func {
            1 => {
                if nums.is_empty() {
                    Value::Error(CellError::Div0)
                } else {
                    Value::Number(nums.iter().sum::<f64>() / nums.len() as f64)
                }
            }
            2 => Value::Number(nums.len() as f64),
            4 => Value::Number(nums.iter().cloned().fold(f64::MIN, f64::max)),
            5 => Value::Number(nums.iter().cloned().fold(f64::MAX, f64::min)),
            6 => Value::Number(nums.iter().product()),
            9 => Value::Number(nums.iter().sum()),
            _ => Value::Error(CellError::Value),
        }
    }

    /// Like [`Self::collect_numbers`] but optionally tolerates error values.
    fn gather_numbers(&mut self, args: &[Expr], skip_errors: bool) -> Result<Vec<f64>, CellError> {
        let mut nums = Vec::new();
        for arg in args {
            for v in self.flatten(arg) {
                match v {
                    Value::Error(e) => {
                        if !skip_errors {
                            return Err(e);
                        }
                    }
                    Value::Empty => {}
                    Value::Number(n) => nums.push(n),
                    Value::Bool(b) => nums.push(if b { 1.0 } else { 0.0 }),
                    Value::Text(t) => {
                        if let Ok(n) = t.trim().parse::<f64>() {
                            nums.push(n);
                        }
                    }
                }
            }
        }
        Ok(nums)
    }

    // --- function helpers ---

    fn numeric_agg(&mut self, args: &[Expr], init: f64, f: fn(f64, f64) -> f64) -> Value {
        match self.collect_numbers(args) {
            Ok(ns) => Value::Number(ns.into_iter().fold(init, f)),
            Err(e) => Value::Error(e),
        }
    }

    fn minmax(&mut self, args: &[Expr], min: bool) -> Value {
        match self.collect_numbers(args) {
            Err(e) => Value::Error(e),
            Ok(ns) if ns.is_empty() => Value::Number(0.0),
            Ok(ns) => {
                let mut acc = ns[0];
                for &n in &ns[1..] {
                    acc = if min { acc.min(n) } else { acc.max(n) };
                }
                Value::Number(acc)
            }
        }
    }

    fn bool_agg(&mut self, args: &[Expr], all: bool) -> Value {
        if args.is_empty() {
            return Value::Error(CellError::Value);
        }
        let mut result = all;
        for arg in args {
            for v in self.flatten(arg) {
                match v.as_bool() {
                    Err(e) => return Value::Error(e),
                    Ok(b) => {
                        if all {
                            result = result && b;
                        } else {
                            result = result || b;
                        }
                    }
                }
            }
        }
        Value::Bool(result)
    }

    fn func_if(&mut self, args: &[Expr]) -> Value {
        if args.len() < 2 || args.len() > 3 {
            return Value::Error(CellError::Value);
        }
        match self.eval(&args[0]).as_bool() {
            Err(e) => Value::Error(e),
            Ok(true) => self.eval(&args[1]),
            Ok(false) => {
                if args.len() == 3 {
                    self.eval(&args[2])
                } else {
                    Value::Bool(false)
                }
            }
        }
    }

    fn unary_math(&mut self, args: &[Expr], f: fn(f64) -> f64) -> Value {
        self.scalar1(args, move |n| Value::Number(f(n)))
    }

    /// One numeric argument.
    fn scalar1(&mut self, args: &[Expr], f: impl Fn(f64) -> Value) -> Value {
        if args.len() != 1 {
            return Value::Error(CellError::Value);
        }
        match self.eval(&args[0]).as_number() {
            Ok(n) => f(n),
            Err(e) => Value::Error(e),
        }
    }

    /// Two numeric arguments.
    fn scalar2(&mut self, args: &[Expr], f: impl Fn(f64, f64) -> Value) -> Value {
        if args.len() != 2 {
            return Value::Error(CellError::Value);
        }
        let a = match self.eval(&args[0]).as_number() {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        let b = match self.eval(&args[1]).as_number() {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        f(a, b)
    }

    /// One text argument.
    fn scalar_text1(&mut self, args: &[Expr], f: impl Fn(&str) -> Value) -> Value {
        if args.len() != 1 {
            return Value::Error(CellError::Value);
        }
        let v = self.eval(&args[0]);
        if let Value::Error(e) = v {
            return Value::Error(e);
        }
        f(&v.as_text())
    }

    fn round_family(&mut self, args: &[Expr], mode: RoundMode) -> Value {
        if args.is_empty() || args.len() > 2 {
            return Value::Error(CellError::Value);
        }
        let n = match self.eval(&args[0]).as_number() {
            Ok(n) => n,
            Err(e) => return Value::Error(e),
        };
        let digits = if args.len() == 2 {
            match self.eval(&args[1]).as_number() {
                Ok(d) => d.trunc() as i32,
                Err(e) => return Value::Error(e),
            }
        } else {
            0
        };
        let factor = 10f64.powi(digits);
        let scaled = n * factor;
        let rounded = match mode {
            RoundMode::Half => scaled.round(),
            RoundMode::Up => scaled.abs().ceil() * scaled.signum(),
            RoundMode::Down => scaled.abs().floor() * scaled.signum(),
        };
        Value::Number(rounded / factor)
    }
}

#[derive(Clone, Copy)]
enum RoundMode {
    Half,
    Up,
    Down,
}

/// Which conditional aggregate an `*IFS` call computes.
#[derive(Clone, Copy)]
enum IfsKind {
    Sum,
    Average,
    Count,
}

/// Normalized string similarity in `[0, 1]`: `1 - editDistance/maxLen`,
/// case-insensitive. Two empty strings are perfectly similar.
fn similarity(a: &str, b: &str) -> f64 {
    let a: Vec<char> = a.to_lowercase().chars().collect();
    let b: Vec<char> = b.to_lowercase().chars().collect();
    let max = a.len().max(b.len());
    if max == 0 {
        return 1.0;
    }
    let dist = levenshtein(&a, &b);
    1.0 - dist as f64 / max as f64
}

/// Levenshtein edit distance between two char slices.
fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, &ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

thread_local! {
    /// Per-thread xorshift state for the volatile RAND family, seeded once from
    /// the wall clock so values differ between runs.
    static RNG_STATE: std::cell::Cell<u64> = std::cell::Cell::new(rng_seed());
}

fn rng_seed() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos() as u64)
        .unwrap_or(0x9E37_79B9_7F4A_7C15);
    nanos | 1 // never zero
}

/// A uniform random `f64` in `[0, 1)`.
fn next_rand() -> f64 {
    RNG_STATE.with(|s| {
        let mut x = s.get();
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        s.set(x);
        (x >> 11) as f64 / (1u64 << 53) as f64
    })
}

fn gcd(a: u64, b: u64) -> u64 {
    if b == 0 {
        a
    } else {
        gcd(b, a % b)
    }
}

/// Guard a computed float result: `#NUM!` if it isn't finite (overflow).
fn finite_num(v: f64) -> Value {
    if v.is_finite() {
        Value::Number(v)
    } else {
        Value::Error(CellError::Num)
    }
}

fn fact_value(n: f64) -> Value {
    let n = n.trunc();
    if n < 0.0 {
        return Value::Error(CellError::Num);
    }
    let mut acc = 1.0;
    let mut i = 2.0;
    while i <= n {
        acc *= i;
        i += 1.0;
    }
    finite_num(acc)
}

fn factdouble_value(n: f64) -> Value {
    let n = n.trunc();
    if n < -1.0 {
        return Value::Error(CellError::Num);
    }
    let mut acc = 1.0;
    let mut i = n;
    while i > 1.0 {
        acc *= i;
        i -= 2.0;
    }
    finite_num(acc)
}

/// COMBIN(n,k) or COMBINA(n,k) (combinations with repetition).
fn combin_value(n: f64, k: f64, with_repetition: bool) -> Value {
    let (n, k) = (n.trunc() as i64, k.trunc() as i64);
    if n < 0 || k < 0 {
        return Value::Error(CellError::Num);
    }
    let (n, k) = if with_repetition {
        (n + k - 1, k)
    } else {
        (n, k)
    };
    if k > n {
        return Value::Error(CellError::Num);
    }
    // Multiplicative formula to limit overflow.
    let k = k.min(n - k);
    let mut acc = 1.0;
    for i in 0..k {
        acc = acc * (n - i) as f64 / (i + 1) as f64;
    }
    finite_num(acc.round())
}

fn permut_value(n: f64, k: f64) -> Value {
    let (n, k) = (n.trunc() as i64, k.trunc() as i64);
    if n < 0 || k < 0 || k > n {
        return Value::Error(CellError::Num);
    }
    let mut acc = 1.0;
    for i in 0..k {
        acc *= (n - i) as f64;
    }
    finite_num(acc)
}

const ROMAN_TABLE: &[(u32, &str)] = &[
    (1000, "M"),
    (900, "CM"),
    (500, "D"),
    (400, "CD"),
    (100, "C"),
    (90, "XC"),
    (50, "L"),
    (40, "XL"),
    (10, "X"),
    (9, "IX"),
    (5, "V"),
    (4, "IV"),
    (1, "I"),
];

fn roman_value(n: f64) -> Value {
    let n = n.trunc();
    if !(0.0..=3999.0).contains(&n) {
        return Value::Error(CellError::Value);
    }
    let mut remaining = n as u32;
    let mut out = String::new();
    for &(value, sym) in ROMAN_TABLE {
        while remaining >= value {
            out.push_str(sym);
            remaining -= value;
        }
    }
    Value::Text(out)
}

fn arabic_value(s: &str) -> Value {
    let s = s.trim().to_ascii_uppercase();
    let digit = |c: char| match c {
        'I' => Some(1),
        'V' => Some(5),
        'X' => Some(10),
        'L' => Some(50),
        'C' => Some(100),
        'D' => Some(500),
        'M' => Some(1000),
        _ => None,
    };
    let vals: Vec<i64> = match s.chars().map(digit).collect::<Option<Vec<_>>>() {
        Some(v) => v,
        None => return Value::Error(CellError::Value),
    };
    let mut total = 0i64;
    for i in 0..vals.len() {
        if i + 1 < vals.len() && vals[i] < vals[i + 1] {
            total -= vals[i];
        } else {
            total += vals[i];
        }
    }
    Value::Number(total as f64)
}

fn to_radix(mut n: u64, radix: u32) -> String {
    if n == 0 {
        return "0".to_string();
    }
    let digits = b"0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    let mut out = Vec::new();
    while n > 0 {
        out.push(digits[(n % radix as u64) as usize]);
        n /= radix as u64;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

/// Return `Number(value)` when `in_domain`, else a `#NUM!` error.
fn domain_num(in_domain: bool, value: f64) -> Value {
    if in_domain {
        Value::Number(value)
    } else {
        Value::Error(CellError::Num)
    }
}

/// Round away from zero to the next even (`even = true`) or odd integer.
fn round_to_parity(n: f64, even: bool) -> f64 {
    if n == 0.0 {
        return 0.0;
    }
    let mut x = n.abs().ceil() as i64;
    if (x % 2 == 0) != even {
        x += 1;
    }
    let r = x as f64;
    if n < 0.0 {
        -r
    } else {
        r
    }
}

/// Match a value against an Excel criteria string/number, e.g. `">5"`, `"<>0"`,
/// `"apple"`, `"a*"` (with `*`/`?` wildcards on equality).
fn criteria_matches(value: &Value, criteria: &Value) -> bool {
    let raw = criteria.as_text();
    let raw = raw.trim();
    let (op, operand) = split_criteria_op(raw);

    if let Ok(n) = operand.parse::<f64>() {
        return match value.as_number() {
            Ok(v) => compare_num(v, op, n),
            Err(_) => false,
        };
    }

    let text = value.as_text();
    match op {
        "<>" => !wildcard_eq(&text, operand),
        "=" | "" => wildcard_eq(&text, operand),
        ">" => text.to_lowercase() > operand.to_lowercase(),
        ">=" => text.to_lowercase() >= operand.to_lowercase(),
        "<" => text.to_lowercase() < operand.to_lowercase(),
        "<=" => text.to_lowercase() <= operand.to_lowercase(),
        _ => false,
    }
}

/// Split a leading comparison operator off a criteria string.
fn split_criteria_op(s: &str) -> (&str, &str) {
    for op in [">=", "<=", "<>", ">", "<", "="] {
        if let Some(rest) = s.strip_prefix(op) {
            return (op, rest.trim());
        }
    }
    ("", s)
}

fn compare_num(v: f64, op: &str, n: f64) -> bool {
    match op {
        "" | "=" => v == n,
        "<>" => v != n,
        ">" => v > n,
        ">=" => v >= n,
        "<" => v < n,
        "<=" => v <= n,
        _ => false,
    }
}

/// Case-insensitive equality with `*` (any run) and `?` (any char) wildcards.
fn wildcard_eq(text: &str, pattern: &str) -> bool {
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    wildcard_match(&t, &p)
}

fn wildcard_match(t: &[char], p: &[char]) -> bool {
    if p.is_empty() {
        return t.is_empty();
    }
    match p[0] {
        '*' => {
            // Match zero or more, then the rest.
            wildcard_match(t, &p[1..]) || (!t.is_empty() && wildcard_match(&t[1..], p))
        }
        '?' => !t.is_empty() && wildcard_match(&t[1..], &p[1..]),
        c => !t.is_empty() && t[0] == c && wildcard_match(&t[1..], &p[1..]),
    }
}

/// Order two values. Numbers compare numerically; text compares
/// case-insensitively; type rank is number < text < bool, matching Excel.
fn compare_values(l: &Value, r: &Value) -> Ordering {
    match (l, r) {
        (Value::Number(a), Value::Number(b)) => a.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Value::Bool(a), Value::Bool(b)) => a.cmp(b),
        (Value::Text(a), Value::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (Value::Empty, Value::Empty) => Ordering::Equal,
        // Empty compares as 0 against numbers, "" against text.
        (Value::Empty, Value::Number(b)) => 0.0_f64.partial_cmp(b).unwrap_or(Ordering::Equal),
        (Value::Number(a), Value::Empty) => a.partial_cmp(&0.0).unwrap_or(Ordering::Equal),
        (Value::Empty, Value::Text(b)) => "".cmp(b.as_str()),
        (Value::Text(a), Value::Empty) => a.as_str().cmp(""),
        // Cross-type: rank by kind.
        _ => type_rank(l).cmp(&type_rank(r)),
    }
}

fn type_rank(v: &Value) -> u8 {
    match v {
        Value::Number(_) | Value::Empty => 0,
        Value::Text(_) => 1,
        Value::Bool(_) => 2,
        Value::Error(_) => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::CellRef;

    fn r(a1: &str) -> CellRef {
        CellRef::parse(a1).unwrap()
    }

    fn sheet_with(cells: &[(&str, &str)]) -> Sheet {
        let mut s = Sheet::new("Sheet1");
        for (addr, input) in cells {
            s.set_input(r(addr), input).unwrap();
        }
        s
    }

    fn val(s: &Sheet, a1: &str) -> Value {
        s.get(r(a1))
    }

    #[test]
    fn sum_over_range() {
        let s = sheet_with(&[("A1", "1"), ("A2", "2"), ("A3", "3"), ("B1", "=SUM(A1:A3)")]);
        assert_eq!(val(&s, "B1"), Value::Number(6.0));
    }

    #[test]
    fn average_and_count() {
        let s = sheet_with(&[
            ("A1", "10"),
            ("A2", "20"),
            ("A3", "text"),
            ("B1", "=AVERAGE(A1:A3)"),
            ("B2", "=COUNT(A1:A3)"),
            ("B3", "=COUNTA(A1:A3)"),
        ]);
        assert_eq!(val(&s, "B1"), Value::Number(15.0));
        assert_eq!(val(&s, "B2"), Value::Number(2.0));
        assert_eq!(val(&s, "B3"), Value::Number(3.0));
    }

    #[test]
    fn if_and_comparisons() {
        let s = sheet_with(&[
            ("A1", "5"),
            ("B1", "=IF(A1>3, \"big\", \"small\")"),
            ("B2", "=IF(A1>10, 1, 0)"),
        ]);
        assert_eq!(val(&s, "B1"), Value::Text("big".into()));
        assert_eq!(val(&s, "B2"), Value::Number(0.0));
    }

    #[test]
    fn division_by_zero_propagates() {
        let s = sheet_with(&[("A1", "=1/0"), ("A2", "=A1+1")]);
        assert_eq!(val(&s, "A1"), Value::Error(CellError::Div0));
        assert_eq!(val(&s, "A2"), Value::Error(CellError::Div0));
    }

    #[test]
    fn math_functions() {
        let s = sheet_with(&[
            ("A1", "=ABS(-4)"),
            ("A2", "=SQRT(9)"),
            ("A3", "=POWER(2,10)"),
            ("A4", "=MOD(7,3)"),
            ("A5", "=ROUND(1.23456, 2)"),
            ("A6", "=SQRT(-1)"),
        ]);
        assert_eq!(val(&s, "A1"), Value::Number(4.0));
        assert_eq!(val(&s, "A2"), Value::Number(3.0));
        assert_eq!(val(&s, "A3"), Value::Number(1024.0));
        assert_eq!(val(&s, "A4"), Value::Number(1.0));
        assert_eq!(val(&s, "A5"), Value::Number(1.23));
        assert_eq!(val(&s, "A6"), Value::Error(CellError::Num));
    }

    #[test]
    fn text_functions_and_concat() {
        let s = sheet_with(&[
            ("A1", "hello"),
            ("B1", "=UPPER(A1)"),
            ("B2", "=LEN(A1)"),
            ("B3", "=A1 & \" world\""),
        ]);
        assert_eq!(val(&s, "B1"), Value::Text("HELLO".into()));
        assert_eq!(val(&s, "B2"), Value::Number(5.0));
        assert_eq!(val(&s, "B3"), Value::Text("hello world".into()));
    }

    #[test]
    fn unknown_function_is_name_error() {
        let s = sheet_with(&[("A1", "=FOOBAR(1)")]);
        assert_eq!(val(&s, "A1"), Value::Error(CellError::Name));
    }

    #[test]
    fn logical_functions() {
        let s = sheet_with(&[
            ("A1", "=AND(TRUE, 1>0)"),
            ("A2", "=OR(FALSE, FALSE)"),
            ("A3", "=NOT(TRUE)"),
        ]);
        assert_eq!(val(&s, "A1"), Value::Bool(true));
        assert_eq!(val(&s, "A2"), Value::Bool(false));
        assert_eq!(val(&s, "A3"), Value::Bool(false));
    }

    #[test]
    fn chained_formula_dependencies() {
        let s = sheet_with(&[
            ("A1", "2"),
            ("A2", "=A1*3"),
            ("A3", "=A2+A1"),
            ("A4", "=A3^2"),
        ]);
        assert_eq!(val(&s, "A4"), Value::Number(64.0));
    }

    #[test]
    fn compare_and_similarity_extensions() {
        let s = sheet_with(&[
            ("A1", "=COMPARE(3,5)"),
            ("A2", "=COMPARE(5,5)"),
            ("A3", "=COMPARE(9,5)"),
            ("A4", "=SIMILARITY(\"kitten\",\"kitten\")"),
            ("A5", "=SIMILARITY(\"kitten\",\"sitting\")"),
            ("A6", "=SIMILARITY(\"abc\",\"xyz\")"),
        ]);
        assert_eq!(val(&s, "A1"), Value::Number(-1.0));
        assert_eq!(val(&s, "A2"), Value::Number(0.0));
        assert_eq!(val(&s, "A3"), Value::Number(1.0));
        assert_eq!(val(&s, "A4"), Value::Number(1.0));
        // kitten->sitting is edit distance 3 over length 7.
        let sim = val(&s, "A5").as_number().unwrap();
        assert!((sim - (1.0 - 3.0 / 7.0)).abs() < 1e-9);
        assert_eq!(val(&s, "A6"), Value::Number(0.0));
    }

    #[test]
    fn ai_function_pending_then_refreshed() {
        // Before a refresh, AI(...) is pending → #N/A.
        let mut s = Sheet::new("Sheet1");
        s.set_formula(r("A1"), "=AI(\"summarize sales\")").unwrap();
        assert_eq!(s.get(r("A1")), Value::Error(CellError::NA));

        // After refreshing through a provider, the cached result is returned.
        s.refresh_ai(&crate::ai::EchoProvider);
        assert_eq!(s.get(r("A1")), Value::Text("AI[summarize sales]".into()));
    }

    #[test]
    fn random_and_statistics() {
        let s = sheet_with(&[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "2"),
            ("A4", "4"),
            ("A5", "100"),
            ("B1", "=MEDIAN(A1:A5)"),
            ("B2", "=MODE(A1:A5)"),
            ("B3", "=GEOMEAN(A1:A4)"),
            ("B4", "=HARMEAN(1,2,4)"),
            ("B5", "=TRIMMEAN(A1:A5, 0.4)"),
            ("B6", "=RAND()"),
            ("B7", "=RANDBETWEEN(1,6)"),
        ]);
        assert_eq!(val(&s, "B1"), Value::Number(2.0)); // sorted 1,2,2,4,100 -> 2
        assert_eq!(val(&s, "B2"), Value::Number(2.0)); // most frequent
        assert!((val(&s, "B3").as_number().unwrap() - 2.0).abs() < 1e-9); // (1*2*2*4)^(1/4)=2
        assert!((val(&s, "B4").as_number().unwrap() - 12.0 / 7.0).abs() < 1e-9);
        // 5 values, 40% trim -> drop 1 from each end -> mean of 2,2,4
        assert!((val(&s, "B5").as_number().unwrap() - 8.0 / 3.0).abs() < 1e-9);
        let rand = val(&s, "B6").as_number().unwrap();
        assert!((0.0..1.0).contains(&rand));
        let between = val(&s, "B7").as_number().unwrap();
        assert!((1.0..=6.0).contains(&between) && between.fract() == 0.0);
    }

    #[test]
    fn integer_and_numeral_functions() {
        let s = sheet_with(&[
            ("A1", "=GCD(12,18)"),
            ("A2", "=LCM(4,6)"),
            ("A3", "=FACT(5)"),
            ("A4", "=COMBIN(5,2)"),
            ("A5", "=PERMUT(5,2)"),
            ("A6", "=QUOTIENT(17,5)"),
            ("A7", "=ROMAN(2024)"),
            ("A8", "=ARABIC(\"MMXXIV\")"),
            ("A9", "=BASE(255,16)"),
            ("A10", "=DECIMAL(\"FF\",16)"),
            ("A11", "=FACTDOUBLE(7)"),
        ]);
        assert_eq!(val(&s, "A1"), Value::Number(6.0));
        assert_eq!(val(&s, "A2"), Value::Number(12.0));
        assert_eq!(val(&s, "A3"), Value::Number(120.0));
        assert_eq!(val(&s, "A4"), Value::Number(10.0));
        assert_eq!(val(&s, "A5"), Value::Number(20.0));
        assert_eq!(val(&s, "A6"), Value::Number(3.0));
        assert_eq!(val(&s, "A7"), Value::Text("MMXXIV".into()));
        assert_eq!(val(&s, "A8"), Value::Number(2024.0));
        assert_eq!(val(&s, "A9"), Value::Text("FF".into()));
        assert_eq!(val(&s, "A10"), Value::Number(255.0));
        assert_eq!(val(&s, "A11"), Value::Number(105.0)); // 7*5*3*1
    }

    #[test]
    fn trig_and_logs() {
        let s = sheet_with(&[
            ("A1", "=SIN(0)"),
            ("A2", "=COS(0)"),
            ("A3", "=DEGREES(PI())"),
            ("A4", "=LN(EXP(1))"),
            ("A5", "=LOG(1000)"),
            ("A6", "=LOG(8,2)"),
            ("A7", "=ASIN(2)"),
            ("A8", "=ATAN2(1,1)"),
        ]);
        assert_eq!(val(&s, "A1"), Value::Number(0.0));
        assert_eq!(val(&s, "A2"), Value::Number(1.0));
        assert_eq!(val(&s, "A3"), Value::Number(180.0));
        assert!((val(&s, "A4").as_number().unwrap() - 1.0).abs() < 1e-9);
        assert!((val(&s, "A5").as_number().unwrap() - 3.0).abs() < 1e-9);
        assert!((val(&s, "A6").as_number().unwrap() - 3.0).abs() < 1e-9);
        assert_eq!(val(&s, "A7"), Value::Error(CellError::Num)); // asin domain
        assert!((val(&s, "A8").as_number().unwrap() - std::f64::consts::FRAC_PI_4).abs() < 1e-9);
    }

    #[test]
    fn rounding_family() {
        let s = sheet_with(&[
            ("A1", "=MROUND(10,3)"),
            ("A2", "=CEILING(2.1,1)"),
            ("A3", "=FLOOR(2.9,1)"),
            ("A4", "=EVEN(3)"),
            ("A5", "=ODD(2)"),
            ("A6", "=CEILING.MATH(4.2)"),
        ]);
        assert_eq!(val(&s, "A1"), Value::Number(9.0));
        assert_eq!(val(&s, "A2"), Value::Number(3.0));
        assert_eq!(val(&s, "A3"), Value::Number(2.0));
        assert_eq!(val(&s, "A4"), Value::Number(4.0));
        assert_eq!(val(&s, "A5"), Value::Number(3.0));
        assert_eq!(val(&s, "A6"), Value::Number(5.0));
    }

    #[test]
    fn sumproduct_subtotal_aggregate() {
        let s = sheet_with(&[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("B1", "4"),
            ("B2", "5"),
            ("B3", "6"),
            ("C1", "=SUMPRODUCT(A1:A3,B1:B3)"), // 1*4+2*5+3*6 = 32
            ("C2", "=SUBTOTAL(9,A1:A3)"),       // SUM = 6
            ("C3", "=SUBTOTAL(1,A1:A3)"),       // AVERAGE = 2
            ("C4", "=AGGREGATE(4,6,A1:A3)"),    // MAX = 3
        ]);
        assert_eq!(val(&s, "C1"), Value::Number(32.0));
        assert_eq!(val(&s, "C2"), Value::Number(6.0));
        assert_eq!(val(&s, "C3"), Value::Number(2.0));
        assert_eq!(val(&s, "C4"), Value::Number(3.0));
    }

    #[test]
    fn conditional_aggregation_functions() {
        let s = sheet_with(&[
            ("A1", "apple"),
            ("B1", "10"),
            ("A2", "banana"),
            ("B2", "20"),
            ("A3", "apricot"),
            ("B3", "30"),
            ("A4", "cherry"),
            ("B4", "40"),
            ("C1", "=SUMIF(B1:B4, \">15\")"),
            ("C2", "=COUNTIF(A1:A4, \"a*\")"),
            ("C3", "=SUMIF(A1:A4, \"a*\", B1:B4)"),
            ("C4", "=AVERAGEIF(B1:B4, \">=20\")"),
            ("C5", "=SUMIFS(B1:B4, A1:A4, \"a*\", B1:B4, \">10\")"),
            ("C6", "=COUNTIFS(B1:B4, \">10\", B1:B4, \"<40\")"),
        ]);
        assert_eq!(val(&s, "C1"), Value::Number(90.0)); // 20+30+40
        assert_eq!(val(&s, "C2"), Value::Number(2.0)); // apple, apricot
        assert_eq!(val(&s, "C3"), Value::Number(40.0)); // 10+30
        assert_eq!(val(&s, "C4"), Value::Number(30.0)); // (20+30+40)/3
        assert_eq!(val(&s, "C5"), Value::Number(30.0)); // apricot only (apple's 10 not >10)
        assert_eq!(val(&s, "C6"), Value::Number(2.0)); // 20, 30
    }

    #[test]
    fn bare_range_formula_spills() {
        let mut s = sheet_with(&[("A1", "1"), ("A2", "2"), ("A3", "3")]);
        s.set_formula(r("C1"), "=A1:A3").unwrap();
        let grid = s.evaluate();
        // The array spills from C1 down into C2 and C3.
        assert_eq!(grid[&(2, 0)], Value::Number(1.0));
        assert_eq!(grid[&(2, 1)], Value::Number(2.0));
        assert_eq!(grid[&(2, 2)], Value::Number(3.0));
    }

    #[test]
    fn array_literal_spills_2d() {
        let mut s = Sheet::new("Sheet1");
        s.set_formula(r("A1"), "={1,2;3,4}").unwrap();
        let grid = s.evaluate();
        assert_eq!(grid[&(0, 0)], Value::Number(1.0));
        assert_eq!(grid[&(1, 0)], Value::Number(2.0));
        assert_eq!(grid[&(0, 1)], Value::Number(3.0));
        assert_eq!(grid[&(1, 1)], Value::Number(4.0));
    }

    #[test]
    fn blocked_spill_reports_spill_error() {
        let mut s = sheet_with(&[("A1", "1"), ("A2", "2"), ("C2", "obstacle")]);
        s.set_formula(r("C1"), "=A1:A2").unwrap();
        let grid = s.evaluate();
        // C2 is occupied, so the spill is blocked.
        assert_eq!(grid[&(2, 0)], Value::Error(CellError::Spill));
        assert_eq!(grid[&(2, 1)], Value::Text("obstacle".into()));
    }

    #[test]
    fn spilled_values_are_referenceable() {
        let mut s = sheet_with(&[("A1", "5"), ("A2", "6")]);
        s.set_formula(r("C1"), "=A1:A2").unwrap(); // spills to C1,C2
        s.set_formula(r("E1"), "=C2+1").unwrap(); // references a spilled cell
        let grid = s.evaluate();
        assert_eq!(grid[&(4, 0)], Value::Number(7.0));
    }

    #[test]
    fn iterative_mode_converges_a_feedback_loop() {
        // A1 = A1/2 + 5 has fixed point 10. Batch eval would report #CIRC!.
        let mut s = Sheet::new("Sheet1");
        s.set_formula(r("A1"), "=A1/2 + 5").unwrap();
        assert_eq!(s.get(r("A1")), Value::Error(CellError::Circular));

        let result = s.evaluate_iterative(100, 1e-9);
        let v = result[&(0, 0)].as_number().unwrap();
        assert!((v - 10.0).abs() < 1e-6, "converged to {v}, expected ~10");
    }

    #[test]
    fn iterative_mode_matches_acyclic_results() {
        let s = sheet_with(&[("A1", "3"), ("A2", "=A1*2"), ("A3", "=A2+A1")]);
        let result = s.evaluate_iterative(50, 1e-9);
        assert_eq!(result[&(0, 1)], Value::Number(6.0));
        assert_eq!(result[&(0, 2)], Value::Number(9.0));
    }

    #[test]
    fn whole_column_and_row_spans_aggregate_used_extent() {
        let s = sheet_with(&[
            ("A1", "1"),
            ("A2", "2"),
            ("A3", "3"),
            ("B1", "10"),
            ("C1", "=SUM(A:A)"),
            ("C2", "=SUM(1:1)"),
        ]);
        // SUM over column A = 1+2+3 = 6.
        assert_eq!(val(&s, "C1"), Value::Number(6.0));
        // SUM over row 1 = A1 + B1 (+ C1 which is a formula = 6) = 1+10+6 = 17.
        assert_eq!(val(&s, "C2"), Value::Number(17.0));
    }

    #[test]
    fn span_as_scalar_is_value_error() {
        let s = sheet_with(&[("A1", "5"), ("B1", "=A:A")]);
        assert_eq!(val(&s, "B1"), Value::Error(CellError::Value));
    }

    #[test]
    fn cross_sheet_reference_resolves() {
        let mut wb = Workbook::empty();
        wb.add_sheet("Data").unwrap();
        let data = wb.sheet_mut("Data").unwrap();
        data.set_input(r("A1"), "10").unwrap();
        data.set_input(r("A2"), "20").unwrap();
        wb.add_sheet("Calc").unwrap();
        wb.sheet_mut("Calc")
            .unwrap()
            .set_formula(r("B1"), "=SUM(Data!A1:A2) + Data!A1")
            .unwrap();

        let all = evaluate_workbook(&wb);
        assert_eq!(all["Calc"][&(1, 0)], Value::Number(40.0));
    }

    #[test]
    fn reference_to_unknown_sheet_is_ref_error() {
        let mut wb = Workbook::new();
        wb.sheet_at_mut(0)
            .unwrap()
            .set_formula(r("A1"), "=Ghost!A1")
            .unwrap();
        let all = evaluate_workbook(&wb);
        assert_eq!(all["Sheet1"][&(0, 0)], Value::Error(CellError::Ref));
    }

    #[test]
    fn cross_sheet_cycle_is_detected() {
        let mut wb = Workbook::empty();
        wb.add_sheet("A").unwrap();
        wb.add_sheet("B").unwrap();
        wb.sheet_mut("A")
            .unwrap()
            .set_formula(r("A1"), "=B!A1")
            .unwrap();
        wb.sheet_mut("B")
            .unwrap()
            .set_formula(r("A1"), "=A!A1")
            .unwrap();
        let all = evaluate_workbook(&wb);
        assert_eq!(all["A"][&(0, 0)], Value::Error(CellError::Circular));
    }

    #[test]
    fn single_sheet_eval_still_works() {
        // A sheet-qualified ref to the sheet's own name resolves; to others, #REF!.
        let mut s = Sheet::new("Main");
        s.set_input(r("A1"), "7").unwrap();
        s.set_formula(r("A2"), "=Main!A1*2").unwrap();
        s.set_formula(r("A3"), "=Other!A1").unwrap();
        assert_eq!(val(&s, "A2"), Value::Number(14.0));
        assert_eq!(val(&s, "A3"), Value::Error(CellError::Ref));
    }
}
