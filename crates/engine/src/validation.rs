//! Data validation: rules that constrain what a cell may contain.
//!
//! A [`Validation`] attaches a [`ValidationRule`] to a range. The rule decides
//! whether a candidate value is acceptable; blanks are always allowed (use a
//! separate "required" check if you need to forbid them).

use crate::address::CellRange;
use crate::value::Value;
use serde::{Deserialize, Serialize};

/// The constraint a validation enforces.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ValidationRule {
    /// Value must equal one of these entries (case-insensitive, trimmed).
    List(Vec<String>),
    /// Value must equal one of the (computed) values in a range — a dropdown
    /// backed by sheet data.
    ListFromRange(CellRange),
    /// Value must be an integer within the optional bounds (inclusive).
    WholeNumber { min: Option<f64>, max: Option<f64> },
    /// Value must be a number within the optional bounds (inclusive).
    Decimal { min: Option<f64>, max: Option<f64> },
    /// Text length must be within the optional bounds (inclusive).
    TextLength {
        min: Option<usize>,
        max: Option<usize>,
    },
}

impl ValidationRule {
    /// Whether `value` satisfies this rule. `resolve_range` supplies the allowed
    /// values for [`ValidationRule::ListFromRange`]; it is not called for other
    /// kinds. Blank values are always accepted.
    pub fn check(&self, value: &Value, resolve_range: &dyn Fn(&CellRange) -> Vec<Value>) -> bool {
        if *value == Value::Empty {
            return true;
        }
        match self {
            ValidationRule::List(items) => {
                let text = value.as_text();
                items
                    .iter()
                    .any(|i| i.trim().eq_ignore_ascii_case(text.trim()))
            }
            ValidationRule::ListFromRange(range) => {
                let text = value.as_text();
                resolve_range(range)
                    .iter()
                    .any(|v| v.as_text().trim().eq_ignore_ascii_case(text.trim()))
            }
            ValidationRule::WholeNumber { min, max } => match value.as_number() {
                Ok(n) => n.fract() == 0.0 && in_bounds(n, *min, *max),
                Err(_) => false,
            },
            ValidationRule::Decimal { min, max } => match value.as_number() {
                Ok(n) => in_bounds(n, *min, *max),
                Err(_) => false,
            },
            ValidationRule::TextLength { min, max } => {
                let len = value.as_text().chars().count();
                min.map(|m| len >= m).unwrap_or(true) && max.map(|m| len <= m).unwrap_or(true)
            }
        }
    }
}

fn in_bounds(n: f64, min: Option<f64>, max: Option<f64>) -> bool {
    min.map(|m| n >= m).unwrap_or(true) && max.map(|m| n <= m).unwrap_or(true)
}

/// A validation rule scoped to a range, with an optional error message shown to
/// the user on rejection.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Validation {
    pub range: CellRange,
    pub rule: ValidationRule,
    pub message: Option<String>,
}

impl Validation {
    pub fn new(range: CellRange, rule: ValidationRule) -> Self {
        Validation {
            range,
            rule,
            message: None,
        }
    }

    /// Builder: attach an error message.
    pub fn with_message(mut self, message: impl Into<String>) -> Self {
        self.message = Some(message.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn no_range(_: &CellRange) -> Vec<Value> {
        Vec::new()
    }

    #[test]
    fn list_validation() {
        let rule = ValidationRule::List(vec!["Yes".into(), "No".into()]);
        assert!(rule.check(&Value::Text("yes".into()), &no_range));
        assert!(!rule.check(&Value::Text("maybe".into()), &no_range));
        // Blank is always allowed.
        assert!(rule.check(&Value::Empty, &no_range));
    }

    #[test]
    fn whole_number_and_decimal() {
        let whole = ValidationRule::WholeNumber {
            min: Some(1.0),
            max: Some(10.0),
        };
        assert!(whole.check(&Value::Number(5.0), &no_range));
        assert!(!whole.check(&Value::Number(5.5), &no_range)); // not whole
        assert!(!whole.check(&Value::Number(11.0), &no_range)); // out of range

        let dec = ValidationRule::Decimal {
            min: Some(0.0),
            max: None,
        };
        assert!(dec.check(&Value::Number(2.5), &no_range));
        assert!(!dec.check(&Value::Number(-1.0), &no_range));
    }

    #[test]
    fn text_length() {
        let rule = ValidationRule::TextLength {
            min: Some(2),
            max: Some(4),
        };
        assert!(rule.check(&Value::Text("abc".into()), &no_range));
        assert!(!rule.check(&Value::Text("a".into()), &no_range));
        assert!(!rule.check(&Value::Text("toolong".into()), &no_range));
    }

    #[test]
    fn list_from_range_uses_resolver() {
        let rule = ValidationRule::ListFromRange(CellRange::parse("A1:A2").unwrap());
        let resolve = |_: &CellRange| vec![Value::Text("Red".into()), Value::Text("Blue".into())];
        assert!(rule.check(&Value::Text("blue".into()), &resolve));
        assert!(!rule.check(&Value::Text("green".into()), &resolve));
    }
}
