//! Property-based tests + a small Excel-parity corpus for the engine.
//!
//! The property tests assert invariants that must hold for *any* input
//! (address round-tripping, parser/unparser idempotency, aggregation equals
//! plain arithmetic). The parity corpus pins a handful of formulas to values
//! hand-verified against a real spreadsheet.

use glasssheet_engine::address::{column_to_index, index_to_column};
use glasssheet_engine::formula;
use glasssheet_engine::{CellRef, Sheet, Value};
use proptest::prelude::*;

/// A recursive strategy producing syntactically-valid arithmetic formulas.
fn arb_formula() -> impl Strategy<Value = String> {
    let leaf = (0i64..1000).prop_map(|n| n.to_string());
    leaf.prop_recursive(4, 48, 4, |inner| {
        let op = prop::sample::select(vec!["+", "-", "*", "/"]);
        prop_oneof![
            (inner.clone(), op, inner.clone()).prop_map(|(a, o, b)| format!("{a}{o}{b}")),
            inner.prop_map(|e| format!("({e})")),
        ]
    })
}

proptest! {
    #[test]
    fn a1_reference_roundtrips(col in 0u32..5000, row in 0u32..5000) {
        let r = CellRef::new(col, row);
        let parsed = CellRef::parse(&r.to_a1()).unwrap();
        prop_assert_eq!((parsed.col, parsed.row), (col, row));
    }

    #[test]
    fn column_letters_roundtrip(idx in 0u32..200_000) {
        let letters = index_to_column(idx);
        prop_assert_eq!(column_to_index(&letters), Some(idx));
    }

    #[test]
    fn unparse_is_idempotent(src in arb_formula()) {
        // parse(unparse(parse(s))) must equal parse(s): the unparser preserves
        // structure exactly (modulo conservative parentheses).
        let ast = formula::parse(&src).unwrap();
        let reparsed = formula::parse(&formula::unparse(&ast)).unwrap();
        prop_assert_eq!(reparsed, ast);
    }

    #[test]
    fn sum_equals_arithmetic_sum(nums in prop::collection::vec(-1000i64..1000, 1..25)) {
        let mut s = Sheet::new("Sheet1");
        for (i, n) in nums.iter().enumerate() {
            s.set_input(CellRef::new(0, i as u32), &n.to_string()).unwrap();
        }
        s.set_formula(CellRef::new(1, 0), format!("=SUM(A1:A{})", nums.len()))
            .unwrap();
        let expected: i64 = nums.iter().sum();
        prop_assert_eq!(s.get(CellRef::new(1, 0)), Value::Number(expected as f64));
    }

    #[test]
    fn evaluation_never_panics(src in arb_formula()) {
        // Whatever the random (valid) formula, evaluating it yields some value
        // without panicking — division by zero etc. surface as cell errors.
        let mut s = Sheet::new("Sheet1");
        s.set_formula(CellRef::new(0, 0), src).unwrap();
        let _ = s.get(CellRef::new(0, 0));
    }
}

/// Hand-verified formula → value cases (a small Excel-parity corpus).
#[test]
fn excel_parity_corpus() {
    let cases: &[(&str, Value)] = &[
        ("=2+3*4", Value::Number(14.0)),
        ("=(2+3)*4", Value::Number(20.0)),
        ("=2^3^2", Value::Number(512.0)), // right-associative
        ("=10/4", Value::Number(2.5)),
        ("=ROUND(2.5,0)", Value::Number(3.0)),
        ("=ROUND(1.23456,2)", Value::Number(1.23)),
        ("=ABS(-7)", Value::Number(7.0)),
        ("=MOD(7,3)", Value::Number(1.0)),
        ("=MAX(1,9,4)", Value::Number(9.0)),
        ("=MIN(1,9,4)", Value::Number(1.0)),
        ("=IF(2>1,\"yes\",\"no\")", Value::Text("yes".into())),
        ("=\"a\"&\"b\"&\"c\"", Value::Text("abc".into())),
        ("=LEN(\"hello\")", Value::Number(5.0)),
        ("=50%", Value::Number(0.5)),
        ("=2<>3", Value::Bool(true)),
    ];
    for (src, expected) in cases {
        let mut s = Sheet::new("Sheet1");
        s.set_formula(CellRef::new(0, 0), *src).unwrap();
        assert_eq!(&s.get(CellRef::new(0, 0)), expected, "formula: {src}");
    }
}
