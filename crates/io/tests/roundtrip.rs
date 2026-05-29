//! Round-trip tests: build a sheet, write it to disk, read it back, and confirm
//! both literals and live formulas survive.

use glasssheet_engine::{CellRef, Sheet, Value};
use glasssheet_io::{export_path, import_path};

fn cell(a1: &str) -> CellRef {
    CellRef::parse(a1).unwrap()
}

fn sample_sheet() -> Sheet {
    let mut s = Sheet::new("Budget");
    s.set_input(cell("A1"), "Widgets").unwrap();
    s.set_input(cell("B1"), "4").unwrap();
    s.set_input(cell("C1"), "2.5").unwrap();
    s.set_formula(cell("D1"), "=B1*C1").unwrap();
    s.set_formula(cell("D2"), "=SUM(D1:D1)").unwrap();
    s.set_input(cell("A3"), "TRUE").unwrap();
    s
}

#[test]
fn xlsx_roundtrip_preserves_values_and_formulas() {
    let dir = std::env::temp_dir();
    let path = dir.join("glasssheet_roundtrip.xlsx");

    let original = sample_sheet();
    export_path(&original, &path).expect("export xlsx");

    let reloaded = import_path(&path).expect("import xlsx");

    // Literal values come back intact.
    assert_eq!(reloaded.get(cell("A1")), Value::Text("Widgets".into()));
    assert_eq!(reloaded.get(cell("B1")), Value::Number(4.0));
    assert_eq!(reloaded.get(cell("A3")), Value::Bool(true));

    // The formula survived as a formula and still computes.
    assert_eq!(reloaded.raw_text(cell("D1")), "=B1*C1");
    assert_eq!(reloaded.get(cell("D1")), Value::Number(10.0));
    assert_eq!(reloaded.get(cell("D2")), Value::Number(10.0));

    let _ = std::fs::remove_file(&path);
}

#[test]
fn csv_roundtrip_writes_computed_values() {
    let dir = std::env::temp_dir();
    let path = dir.join("glasssheet_roundtrip.csv");

    let original = sample_sheet();
    export_path(&original, &path).expect("export csv");

    // CSV holds computed values, so the re-imported D1 is the number 10, and
    // there is no longer a formula behind it.
    let reloaded = import_path(&path).expect("import csv");
    assert_eq!(reloaded.get(cell("D1")), Value::Number(10.0));
    assert_eq!(reloaded.raw_text(cell("D1")), "10");

    let _ = std::fs::remove_file(&path);
}

#[test]
fn unsupported_extension_is_rejected() {
    let dir = std::env::temp_dir();
    let path = dir.join("nope.parquet");
    let err = export_path(&sample_sheet(), &path).unwrap_err();
    assert!(matches!(err, glasssheet_io::IoError::UnsupportedFormat(_)));
}

/// A grid of computed values used by the text-format round-trips.
fn grid_sheet() -> Sheet {
    let mut s = Sheet::new("Data");
    let rows = [["Item", "Qty"], ["Apples", "3"], ["Pears", "5"]];
    for (r, [a, b]) in rows.iter().enumerate() {
        s.set_input(CellRef::new(0, r as u32), a).unwrap();
        s.set_input(CellRef::new(1, r as u32), b).unwrap();
    }
    s
}

#[test]
fn json_roundtrip() {
    let path = std::env::temp_dir().join("glasssheet_rt.json");
    export_path(&grid_sheet(), &path).unwrap();
    let reloaded = import_path(&path).unwrap();
    assert_eq!(reloaded.get(cell("A2")), Value::Text("Apples".into()));
    assert_eq!(reloaded.get(cell("B3")), Value::Number(5.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn markdown_roundtrip() {
    let path = std::env::temp_dir().join("glasssheet_rt.md");
    export_path(&grid_sheet(), &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("| Item | Qty |"));
    assert!(text.contains("| --- | --- |"));
    let reloaded = import_path(&path).unwrap();
    assert_eq!(reloaded.get(cell("A1")), Value::Text("Item".into()));
    assert_eq!(reloaded.get(cell("B2")), Value::Number(3.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn html_roundtrip() {
    let path = std::env::temp_dir().join("glasssheet_rt.html");
    export_path(&grid_sheet(), &path).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("<th>Item</th>"));
    assert!(text.contains("<td>Apples</td>"));
    let reloaded = import_path(&path).unwrap();
    assert_eq!(reloaded.get(cell("A1")), Value::Text("Item".into()));
    assert_eq!(reloaded.get(cell("B3")), Value::Number(5.0));
    let _ = std::fs::remove_file(&path);
}

#[test]
fn html_import_handles_external_tables() {
    let path = std::env::temp_dir().join("glasssheet_ext.html");
    std::fs::write(
        &path,
        "<html><body><table>\
         <tr><th>Name</th><th>Score</th></tr>\
         <tr><td>Ada</td><td>99</td></tr>\
         </table></body></html>",
    )
    .unwrap();
    let s = import_path(&path).unwrap();
    assert_eq!(s.get(cell("A1")), Value::Text("Name".into()));
    assert_eq!(s.get(cell("B2")), Value::Number(99.0));
    let _ = std::fs::remove_file(&path);
}
