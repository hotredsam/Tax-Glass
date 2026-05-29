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

    match ext.as_str() {
        "csv" | "tsv" | "txt" => import_csv(path),
        "json" => import_json(path),
        "md" | "markdown" => import_markdown(path),
        "html" | "htm" => import_html(path),
        e if CALAMINE_EXTS.contains(&e) => import_spreadsheet(path),
        _ => Err(IoError::UnsupportedFormat(ext)),
    }
}

/// Build a sheet from a row-major grid of cell strings, inferring each type.
fn sheet_from_rows(name: &str, rows: &[Vec<String>]) -> Result<Sheet> {
    let mut sheet = Sheet::new(name);
    for (r, row) in rows.iter().enumerate() {
        for (c, field) in row.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            sheet.set_input(CellRef::new(c as u32, r as u32), field)?;
        }
    }
    Ok(sheet)
}

fn stem(path: &Path) -> String {
    path.file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("Sheet1")
        .to_string()
}

/// Import a JSON document of the shape written by `export_json`
/// (`{"rows": [[...], ...]}`) or a bare array of arrays.
pub fn import_json(path: impl AsRef<Path>) -> Result<Sheet> {
    use serde_json::Value as J;
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    let doc: J = serde_json::from_str(&text).map_err(|e| IoError::Csv(e.to_string()))?;
    let (name, rows_json) = match doc {
        J::Object(mut map) => {
            let name = map
                .get("sheet")
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or_else(|| stem(path));
            let rows = map.remove("rows").unwrap_or(J::Array(vec![]));
            (name, rows)
        }
        other => (stem(path), other),
    };
    let arr = rows_json
        .as_array()
        .ok_or_else(|| IoError::Csv("expected an array of rows".into()))?;
    let rows: Vec<Vec<String>> = arr
        .iter()
        .map(|row| {
            row.as_array()
                .map(|cells| cells.iter().map(json_cell_to_string).collect())
                .unwrap_or_default()
        })
        .collect();
    sheet_from_rows(&name, &rows)
}

fn json_cell_to_string(v: &serde_json::Value) -> String {
    use serde_json::Value as J;
    match v {
        J::Null => String::new(),
        J::Bool(b) => b.to_string().to_uppercase(),
        J::Number(n) => n.to_string(),
        J::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// Import a Markdown table (the first table found). The separator row
/// (`|---|`) is skipped.
pub fn import_markdown(path: impl AsRef<Path>) -> Result<Sheet> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in text.lines() {
        let trimmed = line.trim();
        if !trimmed.contains('|') {
            if rows.is_empty() {
                continue;
            } else {
                break; // table ended
            }
        }
        let cells = split_md_row(trimmed);
        // Skip the header separator row (all cells are dashes/colons).
        if cells
            .iter()
            .all(|c| !c.is_empty() && c.chars().all(|ch| ch == '-' || ch == ':'))
        {
            continue;
        }
        rows.push(cells);
    }
    sheet_from_rows(&stem(path), &rows)
}

fn split_md_row(line: &str) -> Vec<String> {
    let line = line.trim().trim_start_matches('|').trim_end_matches('|');
    line.split('|')
        .map(|c| c.trim().replace("\\|", "|"))
        .collect()
}

/// Import the first `<table>` from an HTML document. A lightweight scan handles
/// the tables we (and most tools) emit; it is not a full HTML parser.
pub fn import_html(path: impl AsRef<Path>) -> Result<Sheet> {
    let path = path.as_ref();
    let text = std::fs::read_to_string(path)?;
    let lower = text.to_lowercase();
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut search = 0;
    while let Some(tr_start) = lower[search..].find("<tr").map(|i| i + search) {
        let row_end = lower[tr_start..]
            .find("</tr>")
            .map(|i| i + tr_start)
            .unwrap_or(text.len());
        let row_html = &text[tr_start..row_end];
        let mut cells = Vec::new();
        let lower_row = row_html.to_lowercase();
        let mut pos = 0;
        while let Some(open) = next_cell_open(&lower_row, pos) {
            // Skip to the end of the opening tag.
            let content_start = lower_row[open..].find('>').map(|i| open + i + 1);
            let Some(cs) = content_start else { break };
            let close = lower_row[cs..]
                .find("</td>")
                .or_else(|| lower_row[cs..].find("</th>"))
                .map(|i| cs + i)
                .unwrap_or(row_html.len());
            cells.push(strip_tags(&row_html[cs..close]));
            pos = close + 1;
        }
        if !cells.is_empty() {
            rows.push(cells);
        }
        search = row_end + 5;
    }
    sheet_from_rows(&stem(path), &rows)
}

fn next_cell_open(lower_row: &str, from: usize) -> Option<usize> {
    let td = lower_row[from..].find("<td").map(|i| i + from);
    let th = lower_row[from..].find("<th").map(|i| i + from);
    match (td, th) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// Strip HTML tags and unescape the handful of entities we emit.
fn strip_tags(s: &str) -> String {
    let mut out = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            c if !in_tag => out.push(c),
            _ => {}
        }
    }
    out.trim()
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
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
