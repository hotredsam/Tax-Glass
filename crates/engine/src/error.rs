//! Crate-level error type for parse/build operations.
//!
//! Note: *cell* errors (`#DIV/0!`, `#REF!`, …) are values, not `Result` errors
//! — those live in [`crate::value::CellError`]. `EngineError` is for failures
//! that prevent us from building a workbook at all (bad addresses, malformed
//! formulas the parser rejects).

use thiserror::Error;

/// Errors produced while parsing addresses or formulas.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum EngineError {
    /// An A1-style reference could not be parsed (e.g. `"1A"`, `""`).
    #[error("invalid cell reference: {0:?}")]
    BadReference(String),

    /// A formula failed to tokenize or parse.
    #[error("syntax error in formula: {0}")]
    Syntax(String),

    /// An edit was rejected because the cell is locked on a protected sheet.
    #[error("cell {0} is locked on a protected sheet")]
    Locked(String),
}

/// Convenience alias used throughout the crate.
pub type Result<T> = std::result::Result<T, EngineError>;
