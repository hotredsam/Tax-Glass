//! Formula evaluation over a [`Sheet`].
//!
//! Evaluation is lazy and memoized: each referenced cell is computed on demand
//! and cached, and an in-progress set detects circular references (reported as
//! the `#CIRC!` cell error rather than a panic or infinite loop).

use crate::formula::{BinOp, Expr};
use crate::sheet::{CellContent, Sheet};
use crate::value::{CellError, Value};
use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};

/// Compute the value of every populated cell in the sheet.
pub fn evaluate_sheet(sheet: &Sheet) -> HashMap<(u32, u32), Value> {
    let mut ev = Evaluator {
        sheet,
        cache: HashMap::new(),
        in_progress: HashSet::new(),
    };
    let keys: Vec<(u32, u32)> = sheet.iter().map(|(k, _)| *k).collect();
    for key in keys {
        let v = ev.value_at(key.0, key.1);
        ev.cache.insert(key, v);
    }
    ev.cache
}

struct Evaluator<'a> {
    sheet: &'a Sheet,
    cache: HashMap<(u32, u32), Value>,
    in_progress: HashSet<(u32, u32)>,
}

impl Evaluator<'_> {
    /// The computed value at a coordinate, with memoization and cycle guarding.
    fn value_at(&mut self, col: u32, row: u32) -> Value {
        let key = (col, row);
        if let Some(v) = self.cache.get(&key) {
            return v.clone();
        }
        if self.in_progress.contains(&key) {
            return Value::Error(CellError::Circular);
        }

        let value = match self.sheet.content(col, row) {
            None => Value::Empty,
            Some(CellContent::Literal(v)) => v.clone(),
            Some(CellContent::Formula { ast, .. }) => {
                let ast = ast.clone();
                self.in_progress.insert(key);
                let v = self.eval(&ast);
                self.in_progress.remove(&key);
                v
            }
        };
        self.cache.insert(key, value.clone());
        value
    }

    /// Evaluate an expression to a scalar value.
    fn eval(&mut self, expr: &Expr) -> Value {
        match expr {
            Expr::Number(n) => Value::Number(*n),
            Expr::Text(t) => Value::Text(t.clone()),
            Expr::Bool(b) => Value::Bool(*b),
            Expr::Ref(r) => self.value_at(r.col, r.row),
            // A bare range used as a scalar has no implicit intersection here.
            Expr::Range(_) => Value::Error(CellError::Value),
            Expr::Name(_) => Value::Error(CellError::Name),
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
    /// ranges cell-by-cell.
    fn flatten(&mut self, expr: &Expr) -> Vec<Value> {
        match expr {
            Expr::Range(range) => {
                let mut out = Vec::with_capacity(range.len());
                for cell in range.cells() {
                    out.push(self.value_at(cell.col, cell.row));
                }
                out
            }
            other => vec![self.eval(other)],
        }
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
            "ROUND" => self.round_family(args, RoundMode::Half),
            "ROUNDUP" => self.round_family(args, RoundMode::Up),
            "ROUNDDOWN" => self.round_family(args, RoundMode::Down),

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

            _ => Value::Error(CellError::Name),
        }
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
}
