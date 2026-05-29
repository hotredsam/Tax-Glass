//! # GlassSheet I/O
//!
//! File import and export for [`glasssheet_engine::Sheet`].
//!
//! - **Import:** `.csv`/`.tsv`, and `.xlsx`/`.xlsm`/`.xlsb`/`.xls`/`.ods` via
//!   `calamine`. Stored formulas are preserved where the file provides them.
//! - **Export:** `.csv`/`.tsv` (computed values) and `.xlsx` (a real, editable
//!   workbook — formulas stay live).
//!
//! ```no_run
//! use glasssheet_io::{import_path, export_path};
//!
//! let sheet = import_path("data.xlsx")?;
//! export_path(&sheet, "out.csv")?;
//! # Ok::<(), glasssheet_io::IoError>(())
//! ```

pub mod error;
pub mod export;
pub mod import;

pub use error::{IoError, Result};
pub use export::{export_csv, export_path, export_xlsx};
pub use import::{import_csv, import_path, import_spreadsheet};
