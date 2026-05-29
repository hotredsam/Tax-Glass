//! Conditional formatting: rules that apply a [`CellStyle`] to cells in a range
//! when a condition holds for the cell's computed value.
//!
//! Rules are evaluated in priority order; the first rule that matches a cell
//! supplies its style (mirroring Excel's default "first rule wins" behavior).

use crate::address::CellRange;
use crate::style::CellStyle;
use crate::value::Value;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The test a conditional-formatting rule applies to a cell's value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Condition {
    GreaterThan(f64),
    GreaterOrEqual(f64),
    LessThan(f64),
    LessOrEqual(f64),
    EqualTo(f64),
    NotEqualTo(f64),
    /// Inclusive numeric range `[lo, hi]`.
    Between(f64, f64),
    /// Case-insensitive substring match on the cell's text.
    TextContains(String),
    /// Matches any cell that holds an error value.
    IsError,
}

impl Condition {
    /// Whether this condition holds for a computed value.
    pub fn matches(&self, value: &Value) -> bool {
        match self {
            Condition::IsError => value.is_error(),
            Condition::TextContains(needle) => value
                .as_text()
                .to_lowercase()
                .contains(&needle.to_lowercase()),
            _ => {
                let Ok(n) = value.as_number() else {
                    return false;
                };
                match self {
                    Condition::GreaterThan(t) => n > *t,
                    Condition::GreaterOrEqual(t) => n >= *t,
                    Condition::LessThan(t) => n < *t,
                    Condition::LessOrEqual(t) => n <= *t,
                    Condition::EqualTo(t) => n == *t,
                    Condition::NotEqualTo(t) => n != *t,
                    Condition::Between(lo, hi) => n >= *lo && n <= *hi,
                    _ => false,
                }
            }
        }
    }
}

/// A conditional-formatting rule: a condition over a range, plus the style to
/// apply where it matches.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    pub range: CellRange,
    pub condition: Condition,
    pub style: CellStyle,
}

impl Rule {
    pub fn new(range: CellRange, condition: Condition, style: CellStyle) -> Self {
        Rule {
            range,
            condition,
            style,
        }
    }
}

/// Compute the effective conditional style for each cell, given the rules (in
/// priority order) and the computed value grid. The first matching rule wins.
pub fn effective_styles(
    rules: &[Rule],
    computed: &HashMap<(u32, u32), Value>,
) -> HashMap<(u32, u32), CellStyle> {
    let mut out: HashMap<(u32, u32), CellStyle> = HashMap::new();
    for rule in rules {
        for cell in rule.range.cells() {
            let key = (cell.col, cell.row);
            if out.contains_key(&key) {
                continue; // already claimed by a higher-priority rule
            }
            let value = computed.get(&key).cloned().unwrap_or(Value::Empty);
            if rule.condition.matches(&value) {
                out.insert(key, rule.style.clone());
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::style::Color;

    #[test]
    fn numeric_conditions() {
        assert!(Condition::GreaterThan(10.0).matches(&Value::Number(11.0)));
        assert!(!Condition::GreaterThan(10.0).matches(&Value::Number(10.0)));
        assert!(Condition::Between(1.0, 5.0).matches(&Value::Number(3.0)));
        assert!(!Condition::Between(1.0, 5.0).matches(&Value::Number(6.0)));
        // Non-numeric values never satisfy a numeric condition.
        assert!(!Condition::GreaterThan(0.0).matches(&Value::Text("hi".into())));
    }

    #[test]
    fn text_and_error_conditions() {
        assert!(Condition::TextContains("err".into()).matches(&Value::Text("Server Error".into())));
        assert!(Condition::IsError.matches(&Value::Error(crate::value::CellError::Div0)));
        assert!(!Condition::IsError.matches(&Value::Number(1.0)));
    }

    #[test]
    fn first_matching_rule_wins() {
        let red = CellStyle {
            fill: Some(Color::rgb(255, 0, 0)),
            ..Default::default()
        };
        let green = CellStyle {
            fill: Some(Color::rgb(0, 255, 0)),
            ..Default::default()
        };
        let rules = vec![
            Rule::new(
                CellRange::parse("A1:A3").unwrap(),
                Condition::GreaterThan(5.0),
                red.clone(),
            ),
            Rule::new(
                CellRange::parse("A1:A3").unwrap(),
                Condition::GreaterThan(0.0),
                green,
            ),
        ];
        let mut computed = HashMap::new();
        computed.insert((0, 0), Value::Number(10.0)); // > 5  -> red
        computed.insert((0, 1), Value::Number(3.0)); //  > 0  -> green
        computed.insert((0, 2), Value::Number(-1.0)); // no match

        let styles = effective_styles(&rules, &computed);
        assert_eq!(styles.get(&(0, 0)), Some(&red));
        assert_eq!(
            styles.get(&(0, 1)).unwrap().fill,
            Some(Color::rgb(0, 255, 0))
        );
        assert!(!styles.contains_key(&(0, 2)));
    }
}
