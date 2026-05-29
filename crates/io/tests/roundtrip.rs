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
