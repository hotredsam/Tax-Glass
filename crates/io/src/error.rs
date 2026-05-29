//! Error type for import/export operations.

use thiserror::Error;

/// Failures that can occur reading or writing a spreadsheet file.
#[derive(Debug, Error)]
pub enum IoError {
    /// The file extension isn't one we can read or write.
    #[error("unsupported file format: {0:?}")]
    UnsupportedFormat(String),

    /// The spreadsheet backend (calamine / rust_xlsxwriter) failed.
    #[error("spreadsheet backend error: {0}")]
    Backend(String),

    /// A formula read from the file could not be parsed by the engine.
    #[error("formula parse error: {0}")]
    Formula(#[from] glasssheet_engine::EngineError),

    /// Underlying I/O failure.
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    /// CSV parsing failure.
    #[error("csv error: {0}")]
    Csv(String),
}

impl From<csv::Error> for IoError {
    fn from(e: csv::Error) -> Self {
        IoError::Csv(e.to_string())
    }
}

/// Convenience alias.
pub type Result<T> = std::result::Result<T, IoError>;
