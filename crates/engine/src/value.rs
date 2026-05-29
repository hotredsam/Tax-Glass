//! Cell values and in-cell error values.

use serde::{Deserialize, Serialize};
use std::fmt;

/// A spreadsheet error value, the kind that surfaces *in a cell* (as opposed to
/// a Rust `Result::Err`). These propagate through formulas the way Excel's do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CellError {
    /// Division by zero — `#DIV/0!`.
    Div0,
    /// Wrong type of argument — `#VALUE!`.
    Value,
    /// Invalid cell reference — `#REF!`.
    Ref,
    /// Unknown name or function — `#NAME?`.
    Name,
    /// Numeric overflow / invalid numeric result — `#NUM!`.
    Num,
    /// Value not available — `#N/A`.
    NA,
    /// Circular reference — `#CIRC!` (Excel reports this separately; we model it
    /// as a propagating value).
    Circular,
}

impl CellError {
    /// The canonical spreadsheet spelling of this error.
    pub fn code(&self) -> &'static str {
        match self {
            CellError::Div0 => "#DIV/0!",
            CellError::Value => "#VALUE!",
            CellError::Ref => "#REF!",
            CellError::Name => "#NAME?",
            CellError::Num => "#NUM!",
            CellError::NA => "#N/A",
            CellError::Circular => "#CIRC!",
        }
    }
}

impl fmt::Display for CellError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

/// The value held by, or computed for, a cell.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Value {
    /// A blank cell. Coerces to `0` in arithmetic and `""` in text contexts.
    Empty,
    /// A floating-point number. Booleans become 1/0 when used numerically.
    Number(f64),
    /// Text.
    Text(String),
    /// A boolean (`TRUE`/`FALSE`).
    Bool(bool),
    /// An error value that propagates through dependent formulas.
    Error(CellError),
}

impl Value {
    /// Coerce to a number for arithmetic. Blanks are `0`; booleans are `1`/`0`;
    /// numeric text parses; anything else is a `#VALUE!` error.
    pub fn as_number(&self) -> Result<f64, CellError> {
        match self {
            Value::Empty => Ok(0.0),
            Value::Number(n) => Ok(*n),
            Value::Bool(b) => Ok(if *b { 1.0 } else { 0.0 }),
            Value::Text(t) => t.trim().parse::<f64>().map_err(|_| CellError::Value),
            Value::Error(e) => Err(*e),
        }
    }

    /// Coerce to text for concatenation and display.
    pub fn as_text(&self) -> String {
        match self {
            Value::Empty => String::new(),
            Value::Number(n) => format_number(*n),
            Value::Text(t) => t.clone(),
            Value::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
            Value::Error(e) => e.code().to_string(),
        }
    }

    /// Truthiness for logical functions: `0`/empty/`FALSE`/`""` are falsey.
    pub fn as_bool(&self) -> Result<bool, CellError> {
        match self {
            Value::Empty => Ok(false),
            Value::Bool(b) => Ok(*b),
            Value::Number(n) => Ok(*n != 0.0),
            Value::Text(t) => match t.trim().to_ascii_uppercase().as_str() {
                "TRUE" => Ok(true),
                "FALSE" | "" => Ok(false),
                _ => Err(CellError::Value),
            },
            Value::Error(e) => Err(*e),
        }
    }

    /// Whether this value is an error.
    pub fn is_error(&self) -> bool {
        matches!(self, Value::Error(_))
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.as_text())
    }
}

/// Render a number without a trailing `.0`, but keep genuine fractions. Mirrors
/// the "general" number format closely enough for display and CSV output.
pub fn format_number(n: f64) -> String {
    if n == n.trunc() && n.abs() < 1e15 {
        format!("{}", n as i64)
    } else {
        // Trim to a reasonable precision and strip trailing zeros.
        let s = format!("{n:.10}");
        let trimmed = s.trim_end_matches('0').trim_end_matches('.');
        trimmed.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numeric_coercion() {
        assert_eq!(Value::Empty.as_number(), Ok(0.0));
        assert_eq!(Value::Bool(true).as_number(), Ok(1.0));
        assert_eq!(Value::Text("3.5".into()).as_number(), Ok(3.5));
        assert_eq!(Value::Text("abc".into()).as_number(), Err(CellError::Value));
        assert_eq!(
            Value::Error(CellError::Div0).as_number(),
            Err(CellError::Div0)
        );
    }

    #[test]
    fn text_rendering() {
        assert_eq!(Value::Number(42.0).as_text(), "42");
        assert_eq!(Value::Number(3.5).as_text(), "3.5");
        assert_eq!(Value::Bool(false).as_text(), "FALSE");
        assert_eq!(Value::Error(CellError::NA).as_text(), "#N/A");
        assert_eq!(Value::Empty.as_text(), "");
    }

    #[test]
    fn number_formatting_strips_trailing_zeros() {
        assert_eq!(format_number(1.0), "1");
        assert_eq!(format_number(1.5), "1.5");
        assert_eq!(format_number(-0.25), "-0.25");
        assert_eq!(format_number(1000000.0), "1000000");
    }

    #[test]
    fn boolean_coercion() {
        assert_eq!(Value::Number(0.0).as_bool(), Ok(false));
        assert_eq!(Value::Number(2.0).as_bool(), Ok(true));
        assert_eq!(Value::Text("TRUE".into()).as_bool(), Ok(true));
        assert_eq!(Value::Empty.as_bool(), Ok(false));
    }
}
