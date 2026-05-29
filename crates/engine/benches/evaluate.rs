//! Performance benchmarks for the engine. Run with `cargo bench`.
//!
//! These guard the "faster than a spreadsheet you'd actually wait on" goal:
//! a 10k-deep dependency chain evaluates in well under a millisecond on a
//! modern machine, and an incremental edit recomputes only what changed.

use criterion::{black_box, criterion_group, criterion_main, Criterion};
use glasssheet_engine::{formula, CellRef, RecalcEngine, Sheet, Value};

/// Build a sheet with an `n`-deep chain: A1 = 1, A2 = A1+1, … An = A(n-1)+1.
fn build_chain(n: u32) -> Sheet {
    let mut s = Sheet::new("Bench");
    s.set_input(CellRef::new(0, 0), "1").unwrap();
    for row in 1..n {
        s.set_formula(CellRef::new(0, row), format!("=A{}+1", row))
            .unwrap();
    }
    s
}

fn bench_full_evaluate(c: &mut Criterion) {
    let sheet = build_chain(10_000);
    c.bench_function("evaluate_10k_chain", |b| {
        b.iter(|| black_box(sheet.evaluate()))
    });
}

fn bench_parse(c: &mut Criterion) {
    c.bench_function("parse_formula", |b| {
        b.iter(|| black_box(formula::parse("SUM(A1:A100) * (B2 + 3) / MAX(C1:C9)").unwrap()))
    });
}

fn bench_incremental_edit(c: &mut Criterion) {
    // Editing a leaf cell (nothing depends on it) in a warm 10k-cell sheet:
    // the incremental recalculator touches only that one cell.
    let mut engine = RecalcEngine::new(build_chain(10_000));
    engine.recalculate(); // warm the cache
    let leaf = CellRef::new(0, 9_999);
    c.bench_function("incremental_leaf_edit_in_10k", |b| {
        let mut v = 1.0;
        b.iter(|| {
            v += 1.0;
            engine.set_value(leaf, Value::Number(v));
            black_box(engine.get(leaf));
        })
    });
}

fn bench_aggregate(c: &mut Criterion) {
    // SUM over a 50k-cell column exercises the streaming gather + kernel reduce.
    let mut s = Sheet::new("Agg");
    for row in 0..50_000u32 {
        s.set_input(CellRef::new(0, row), &row.to_string()).unwrap();
    }
    s.set_formula(CellRef::new(1, 0), "=SUM(A1:A50000)")
        .unwrap();
    c.bench_function("sum_over_50k", |b| b.iter(|| black_box(s.evaluate())));
}

criterion_group!(
    benches,
    bench_full_evaluate,
    bench_parse,
    bench_incremental_edit,
    bench_aggregate
);
criterion_main!(benches);
