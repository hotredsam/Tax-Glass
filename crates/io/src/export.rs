//! Exporting an engine [`Sheet`] to a file.
//!
//! CSV export writes *computed* values (what you'd see on screen). XLSX export
//! writes a real, editable workbook: literal cells as typed values and formula
//! cells as live formulas.

use crate::error::{IoError, Result};
use glasssheet_engine::{CellContent, Sheet, Value};
use rust_xlsxwriter::{Formula, Workbook};
use std::path::Path;

/// Export by extension: `.csv`/`.tsv` → CSV/TSV, `.xlsx` → workbook.
pub fn export_path(sheet: &Sheet, path: impl AsRef<Path>) -> Result<()> {
    let path = path.as_ref();
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "csv" => export_csv(sheet, path, b','),
        "tsv" => export_csv(sheet, path, b'\t'),
        "xlsx" => export_xlsx(sheet, path),
        "json" => export_json(sheet, path),
        "md" | "markdown" => export_markdown(sheet, path),
        "html" | "htm" => export_html(sheet, path),
        _ => Err(IoError::UnsupportedFormat(ext)),
    }
}

/// The computed value grid as a dense `rows × cols` matrix.
fn computed_grid(sheet: &Sheet) -> Vec<Vec<Value>> {
    let (cols, rows) = sheet.dimensions();
    let computed = sheet.evaluate();
    (0..rows)
        .map(|r| {
            (0..cols)
                .map(|c| computed.get(&(c, r)).cloned().unwrap_or(Value::Empty))
                .collect()
        })
        .collect()
}

/// Export computed values as JSON: an array of rows, each an array of cells
/// (numbers as numbers, booleans as booleans, blanks as `null`, everything else
/// as strings).
pub fn export_json(sheet: &Sheet, path: impl AsRef<Path>) -> Result<()> {
    use serde_json::{Map, Value as J};
    let grid = computed_grid(sheet);
    let rows: Vec<J> = grid
        .iter()
        .map(|row| J::Array(row.iter().map(value_to_json).collect()))
        .collect();
    let doc = J::Object(Map::from_iter([
        ("sheet".to_string(), J::String(sheet.name.clone())),
        ("rows".to_string(), J::Array(rows)),
    ]));
    let text = serde_json::to_string_pretty(&doc).map_err(|e| IoError::Backend(e.to_string()))?;
    std::fs::write(path.as_ref(), text)?;
    Ok(())
}

fn value_to_json(v: &Value) -> serde_json::Value {
    use serde_json::Value as J;
    match v {
        Value::Empty => J::Null,
        Value::Number(n) => serde_json::Number::from_f64(*n)
            .map(J::Number)
            .unwrap_or(J::Null),
        Value::Bool(b) => J::Bool(*b),
        Value::Text(t) => J::String(t.clone()),
        Value::Error(e) => J::String(e.code().to_string()),
    }
}

/// Export computed values as a GitHub-flavored Markdown table (row 0 is the
/// header row).
pub fn export_markdown(sheet: &Sheet, path: impl AsRef<Path>) -> Result<()> {
    let grid = computed_grid(sheet);
    let cols = grid.first().map(|r| r.len()).unwrap_or(0);
    let mut out = String::new();
    if cols == 0 {
        std::fs::write(path.as_ref(), out)?;
        return Ok(());
    }
    let esc = |v: &Value| v.as_text().replace('|', "\\|").replace('\n', " ");
    let row_line = |cells: &[Value]| {
        let mut s = String::from("|");
        for c in cells {
            s.push(' ');
            s.push_str(&esc(c));
            s.push_str(" |");
        }
        s.push('\n');
        s
    };

    let empty = vec![Value::Empty; cols];
    out.push_str(&row_line(grid.first().unwrap_or(&empty)));
    out.push('|');
    for _ in 0..cols {
        out.push_str(" --- |");
    }
    out.push('\n');
    for row in grid.iter().skip(1) {
        out.push_str(&row_line(row));
    }
    std::fs::write(path.as_ref(), out)?;
    Ok(())
}

/// Export computed values as an HTML `<table>` (row 0 becomes `<th>` headers).
pub fn export_html(sheet: &Sheet, path: impl AsRef<Path>) -> Result<()> {
    let grid = computed_grid(sheet);
    let esc = |v: &Value| {
        v.as_text()
            .replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    };
    let mut out = String::from("<table>\n");
    for (i, row) in grid.iter().enumerate() {
        let tag = if i == 0 { "th" } else { "td" };
        out.push_str("  <tr>");
        for cell in row {
            out.push_str(&format!("<{tag}>{}</{tag}>", esc(cell)));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</table>\n");
    std::fs::write(path.as_ref(), out)?;
    Ok(())
}

/// Write computed values as a delimited text file.
pub fn export_csv(sheet: &Sheet, path: impl AsRef<Path>, delimiter: u8) -> Result<()> {
    let (cols, rows) = sheet.dimensions();
    let computed = sheet.evaluate();

    let mut writer = csv::WriterBuilder::new()
        .delimiter(delimiter)
        .from_path(path.as_ref())?;

    for row in 0..rows {
        let mut record: Vec<String> = Vec::with_capacity(cols as usize);
        for col in 0..cols {
            let text = computed
                .get(&(col, row))
                .cloned()
                .unwrap_or(Value::Empty)
                .as_text();
            record.push(text);
        }
        writer.write_record(&record)?;
    }
    writer.flush()?;
    Ok(())
}

/// Write a real `.xlsx` workbook: literals as typed values, formulas as live
/// formulas so the file stays editable in Excel/LibreOffice.
pub fn export_xlsx(sheet: &Sheet, path: impl AsRef<Path>) -> Result<()> {
    let mut workbook = Workbook::new();
    let worksheet = workbook.add_worksheet();
    worksheet
        .set_name(clamp_sheet_name(&sheet.name))
        .map_err(|e| IoError::Backend(e.to_string()))?;

    for (&(col, row), content) in sheet.iter() {
        let (r, c) = (row, col as u16);
        let res = match content {
            CellContent::Literal(Value::Number(n)) => worksheet.write_number(r, c, *n).map(|_| ()),
            CellContent::Literal(Value::Text(t)) => worksheet.write_string(r, c, t).map(|_| ()),
            CellContent::Literal(Value::Bool(b)) => worksheet.write_boolean(r, c, *b).map(|_| ()),
            CellContent::Literal(Value::Error(e)) => {
                worksheet.write_string(r, c, e.code()).map(|_| ())
            }
            CellContent::Literal(Value::Empty) => Ok(()),
            CellContent::Formula { src, .. } => worksheet
                .write_formula(r, c, Formula::new(format!("={src}")))
                .map(|_| ()),
        };
        res.map_err(|e| IoError::Backend(e.to_string()))?;
    }

    workbook
        .save(path.as_ref())
        .map_err(|e| IoError::Backend(e.to_string()))?;
    Ok(())
}

/// Excel sheet names are capped at 31 chars and forbid a handful of characters.
fn clamp_sheet_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| if "[]:*?/\\".contains(c) { '_' } else { c })
        .collect();
    let trimmed: String = cleaned.chars().take(31).collect();
    if trimmed.is_empty() {
        "Sheet1".to_string()
    } else {
        trimmed
    }
}
