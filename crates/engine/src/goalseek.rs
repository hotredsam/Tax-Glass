//! Goal Seek: find the input value that makes a formula reach a target.
//!
//! Given a `target` formula cell, a desired `target_value`, and a `variable`
//! cell that the formula (transitively) depends on, this searches for the
//! variable value that drives the formula to the target, using the secant
//! method with a bisection-free convergence guard.

use crate::address::CellRef;
use crate::sheet::Sheet;
use crate::value::Value;

/// The outcome of a successful goal-seek.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GoalSeekResult {
    /// The variable value found.
    pub value: f64,
    /// The target formula's value at that input.
    pub achieved: f64,
    /// Iterations used.
    pub iterations: u32,
}

/// Solve for `variable` so that `target` evaluates to `target_value`.
///
/// Returns `None` if the search fails to converge within `max_iterations` or the
/// target cannot be evaluated to a number.
pub fn goal_seek(
    sheet: &Sheet,
    target: CellRef,
    target_value: f64,
    variable: CellRef,
    max_iterations: u32,
    tolerance: f64,
) -> Option<GoalSeekResult> {
    let mut work = sheet.clone();

    // f(x) = target(x) - target_value
    let eval = |work: &mut Sheet, x: f64| -> Option<f64> {
        work.set_value(variable, Value::Number(x));
        work.get(target).as_number().ok().map(|y| y - target_value)
    };

    let start = sheet.get(variable).as_number().unwrap_or(0.0);
    let mut x0 = start;
    let mut x1 = if start == 0.0 { 1.0 } else { start * 1.1 + 1.0 };
    let mut f0 = eval(&mut work, x0)?;

    for it in 1..=max_iterations {
        if f0.abs() <= tolerance {
            return Some(GoalSeekResult {
                value: x0,
                achieved: f0 + target_value,
                iterations: it - 1,
            });
        }
        let f1 = eval(&mut work, x1)?;
        if f1.abs() <= tolerance {
            return Some(GoalSeekResult {
                value: x1,
                achieved: f1 + target_value,
                iterations: it,
            });
        }
        let denom = f1 - f0;
        if denom.abs() < 1e-12 {
            return None; // flat — can't make secant progress
        }
        let x2 = x1 - f1 * (x1 - x0) / denom;
        if !x2.is_finite() {
            return None;
        }
        x0 = x1;
        f0 = f1;
        x1 = x2;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cell(a1: &str) -> CellRef {
        CellRef::parse(a1).unwrap()
    }

    #[test]
    fn solves_linear_target() {
        // A2 = A1 * 2; find A1 so A2 == 10  -> A1 == 5.
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_formula(cell("A2"), "=A1*2").unwrap();
        let r = goal_seek(&s, cell("A2"), 10.0, cell("A1"), 100, 1e-9).unwrap();
        assert!((r.value - 5.0).abs() < 1e-6);
        assert!((r.achieved - 10.0).abs() < 1e-6);
    }

    #[test]
    fn solves_nonlinear_target() {
        // A2 = A1^2 + A1; find A1 so A2 == 6 -> A1 == 2.
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_formula(cell("A2"), "=A1^2 + A1").unwrap();
        let r = goal_seek(&s, cell("A2"), 6.0, cell("A1"), 100, 1e-9).unwrap();
        assert!((r.value - 2.0).abs() < 1e-4, "got {}", r.value);
    }

    #[test]
    fn does_not_modify_the_original_sheet() {
        let mut s = Sheet::new("Sheet1");
        s.set_input(cell("A1"), "1").unwrap();
        s.set_formula(cell("A2"), "=A1*2").unwrap();
        let _ = goal_seek(&s, cell("A2"), 10.0, cell("A1"), 100, 1e-9);
        assert_eq!(s.get(cell("A1")), Value::Number(1.0));
    }
}
