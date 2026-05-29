# GlassSheet

A spreadsheet application written in Rust — a fast, native "better Excel." The
project is a Cargo workspace: a pure-Rust calculation **engine** plus front-ends
that build on it (currently a CLI; desktop/web/mobile planned).

> This repo was previously a browser-based tax tool ("TaxGlass Pro"). It has been
> rewritten in Rust as a general-purpose spreadsheet app. There is **no tax
> logic** anymore — do not reintroduce it.

## Tech Stack

| Technology | Purpose                                            |
|------------|----------------------------------------------------|
| Rust 2021  | Everything                                         |
| Cargo      | Workspace build, test, lint                        |
| `clap`     | CLI argument parsing                               |
| `csv`      | CSV import/export in the CLI                       |
| `serde`    | Serialization of the value/address model           |
| `thiserror`| Error types                                        |

No Node, no npm, no CDNs. Build with `cargo`.

## Workspace layout

```
Cargo.toml              # workspace manifest + shared dependency versions
crates/
  engine/               # glasssheet-engine (library) — the calculation core
    src/value.rs        #   Value + CellError (in-cell #DIV/0! etc.)
    src/address.rs      #   CellRef / CellRange + A1 <-> index conversion
    src/formula.rs      #   tokenizer, Expr AST, recursive-descent parser
    src/eval.rs         #   memoized evaluator, built-in functions, cycle guard
    src/sheet.rs        #   Sheet: sparse cell grid, set_input/set_formula
    src/error.rs        #   EngineError (parse/build failures)
    src/lib.rs          #   public re-exports
  cli/                  # glasssheet-cli (binary `glasssheet`)
    src/main.rs         #   CSV load -> evaluate -> table/CSV output
examples/budget.csv     # sample spreadsheet with formulas
```

## How to run

```console
cargo build
cargo test                       # unit + doc tests
cargo clippy --all-targets       # lint (CI runs with -D warnings)
cargo fmt                        # format (CI runs --check)
cargo run -p glasssheet-cli -- examples/budget.csv
```

## Architecture notes

- **Values vs. errors.** A `Result::Err` (`EngineError`) means we couldn't build
  the workbook (bad address/formula syntax). An *in-cell* error
  (`Value::Error(CellError::…)`) is a value that propagates through formulas,
  exactly like Excel's `#DIV/0!`. Don't conflate the two.
- **Addressing is zero-based internally** (`CellRef { col, row }`); A1 syntax is
  only the surface form. `$` absolute markers are preserved but don't affect
  storage identity (cells are keyed by `(col, row)`).
- **Evaluation is lazy + memoized** (`eval::evaluate_sheet`). An `in_progress`
  set detects circular references and yields `CellError::Circular` instead of
  recursing forever. There is no dependency graph yet — recompute is whole-sheet.
- **`Sheet::set_input`** is the type-inference entry point: leading `=` →
  formula, `TRUE`/`FALSE` → bool, numeric string → number, else text. Use it for
  imported/user data. `set_value`/`set_formula` are the typed paths.

## Adding a built-in function

1. Add a match arm in `eval.rs::eval_func` (function names are upper-cased before
   dispatch — match on the upper-case name).
2. Use the helpers: `collect_numbers` (range-flattening aggregation),
   `scalar1`/`scalar2` (fixed numeric arity), `scalar_text1`, `flatten` (expand a
   range arg to values). Propagate `Value::Error` early.
3. Add a unit test in `eval.rs::tests` driving it through `set_input` + `get`.
4. Document it in the README's function list.

## Conventions / gotchas

- Keep the engine free of I/O and UI. File formats and presentation live in
  front-end crates, not in `glasssheet-engine`.
- Numbers render via `value::format_number` (no trailing `.0`, trims zeros).
  Don't `format!("{}", n)` numbers ad hoc — reuse it for consistent output.
- CSV cannot carry a comma inside an unquoted formula; quoted fields are required
  for formulas like `"=ROUND(AVERAGE(A1:A3),2)"`. This is plain CSV semantics.
- CI (`.github/workflows/ci.yml`) gates on `fmt --check`, `clippy -D warnings`,
  and `cargo test --all`. Run all three locally before pushing.
- There are no integration/GUI tests yet; logic is covered by `#[cfg(test)]`
  modules in each engine file.
