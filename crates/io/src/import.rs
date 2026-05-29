//! Importing spreadsheets into an engine [`Sheet`].
//!
//! CSV is read directly (cells starting with `=` become formulas). The binary
//! and OpenDocument formats are read via `calamine`, preserving formulas where
//! the file stores them and falling back to the cached/computed value otherwise.

use crate::error::{IoError, Result};
use calamine::{open_workbook_auto, Data, Reader};
use glasssheet_engine::{CellRef, Sheet, Value};
use std::path::Path;

/// Spreadsheet formats `calamine` can read for us.
const CALAMINE_EXTS: &[&str] = &["xlsx", "xlsm", "xlsb", "xls", "xla", "xlam", "ods"];

/// Import the first worksheet of a file into a [`Sheet`], choosing the reader by
/// extension.
pub fn import_path(path: impl AsRef<Path>) -> Result<Sheet> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    if ext == "csv" || ext == "tsv" || ext == "txt" {
        import_csv(path)
    } else if CALAMINE_EXTS.contains(&ext.as_str()) {
        import_spreadsheet(path)
    } else {
        Err(IoError::UnsupportedFormat(ext))
    }
}

/// Read a CSV/TSV file into a sheet, inferring each cell's type.
pub fn import_csv(path: impl AsRef<Path>) -> Result<Sheet> {
    let path = path.as_ref();
    let delimiter = if path.extension().and_then(|e| e.to_str()) == Some("tsv") {
        b'\t'
    } else {
        b','
    };

    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delimiter)
        .from_path(path)?;

    let name = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sheet1")
        .to_string();
    let mut sheet = Sheet::new(name);

    for (row_idx, record) in reader.records().enumerate() {
        let record = record?;
        for (col_idx, field) in record.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            sheet.set_input(CellRef::new(col_idx as u32, row_idx as u32), field)?;
        }
    }
    Ok(sheet)
}

/// Read the first worksheet of a binary/ODS spreadsheet via calamine.
pub fn import_spreadsheet(path: impl AsRef<Path>) -> Result<Sheet> {
    let mut workbook =
        open_workbook_auto(path.as_ref()).map_err(|e| IoError::Backend(e.to_string()))?;

    let name = workbook
        .sheet_names()
        .first()
        .cloned()
        .ok_or_else(|| IoError::Backend("workbook contains no sheets".into()))?;

    let range = workbook
        .worksheet_range(&name)
        .map_err(|e| IoError::Backend(e.to_string()))?;
    // Formulas are best-effort: not every backend/file provides them.
    let formulas = workbook.worksheet_formula(&name).ok();

    let mut sheet = Sheet::new(name);
    let (start_row, start_col) = range.start().unwrap_or((0, 0));

    for (r, row) in range.rows().enumerate() {
        for (c, data) in row.iter().enumerate() {
            let abs_row = start_row + r as u32;
            let abs_col = start_col + c as u32;
            let cell = CellRef::new(abs_col, abs_row);

            // Prefer a stored formula if present and non-empty.
            let formula = formulas
                .as_ref()
                .and_then(|f| f.get_value((abs_row, abs_col)))
                .filter(|s| !s.is_empty());

            if let Some(src) = formula {
                sheet.set_formula(cell, src.as_str())?;
            } else if let Some(value) = data_to_value(data) {
                sheet.set_value(cell, value);
            }
        }
    }

    Ok(sheet)
}

/// Convert a calamine cell into an engine [`Value`]. Returns `None` for blanks.
fn data_to_value(data: &Data) -> Option<Value> {
    match data {
        Data::Empty => None,
        Data::Int(i) => Some(Value::Number(*i as f64)),
        Data::Float(f) => Some(Value::Number(*f)),
        Data::Bool(b) => Some(Value::Bool(*b)),
        Data::String(s) if s.is_empty() => None,
        Data::String(s) => Some(Value::Text(s.clone())),
        Data::DateTime(dt) => Some(Value::Number(dt.as_f64())),
        Data::DateTimeIso(s) => Some(Value::Text(s.clone())),
        Data::DurationIso(s) => Some(Value::Text(s.clone())),
        Data::Error(e) => Some(Value::Text(format!("{e:?}"))),
    }
}
