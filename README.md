# GlassSheet

A spreadsheet application written in Rust — a fast, native "better Excel."

GlassSheet is built around a pure-Rust calculation engine: a cell/value model,
A1-style addressing, and a formula parser/evaluator with the functions you
expect from a spreadsheet. The engine is the foundation; native desktop, web,
and mobile front-ends are built on top of it.

> **History:** this repository began life as a browser-based tax form tool. It
> has been rewritten from scratch in Rust as a general-purpose spreadsheet
> application. All of the old tax-specific logic has been removed.

## Workspace layout

```
glasssheet/
  Cargo.toml            # workspace
  crates/
    engine/             # glasssheet-engine — the calculation core (library)
      src/
        value.rs        #   cell values + in-cell errors (#DIV/0!, #REF!, …)
        address.rs      #   A1 addressing (CellRef, CellRange)
        formula.rs      #   tokenizer, AST, recursive-descent parser
        eval.rs         #   memoized evaluator + built-in functions
        sheet.rs        #   the worksheet (sparse cell grid)
    io/                 # glasssheet-io — file import/export (csv, xlsx, ods, …)
    cli/                # glasssheet-cli — command-line front-end (binary)
    tui/                # glasssheet-tui — full-screen terminal editor (binary)
  examples/
    budget.csv          # sample spreadsheet with formulas
```

## Terminal UI

A full-screen interactive editor built on the engine:

```console
$ cargo run -p glasssheet-tui                 # blank workbook
$ cargo run -p glasssheet-tui examples/budget.csv
```

```
D2  =B2*C2
         A         B         C         D         E
   1 Item      Qty       Price
   2 Widgets           4       2.5        10
   3 Gadgets           3      9.99     29.97
   4 Gizmos           10      1.25      12.5
   5                                   52.47
 ⠋ NORMAL  GlassSheet — arrows to move, Enter to edit, : for commands
 Sheet1
```

- **Move** with arrow keys (Shift-arrow extends a selection; the status bar
  shows Sum/Avg/Count). **Type** to edit a cell, **Enter** to commit, **Esc** to
  cancel; **Delete** clears.
- **Undo/redo** with Ctrl-Z / Ctrl-Y. **Tab / Shift-Tab** switch sheets.
- **Commands** (press `:`): `:w file`, `:e file`, `:new [name]`, `:sheet name`,
  `:theme Light|Dark|Glass`, `:q`.
- Theme-aware colors, conditional-formatting highlights, and a subtly animated
  cursor (~60 fps).

## Quick start

Build and run the CLI against a CSV. Any cell whose text starts with `=` is a
formula:

```console
$ cargo run -p glasssheet-cli -- examples/budget.csv
  |    A    |  B  |      C      |   D
--+---------+-----+-------------+-------
1 |    Item | Qty |       Price | Total
2 | Widgets |   4 |         2.5 |    10
3 | Gadgets |   3 |        9.99 | 29.97
4 |  Gizmos |  10 |        1.25 |  12.5
5 |         |     | Grand Total | 52.47
6 |         |     |     Average | 17.49
```

Other modes:

```console
$ cargo run -p glasssheet-cli -- examples/budget.csv --format csv   # computed CSV
$ cargo run -p glasssheet-cli -- examples/budget.csv --cell D5      # one cell
$ cargo run -p glasssheet-cli -- examples/budget.csv --cell D5 --raw  # show =SUM(...)
```

### File formats

GlassSheet opens spreadsheets in any of these formats and detects the type from
the file:

| Format | Read | Write |
|--------|:----:|:-----:|
| `.csv` / `.tsv` | ✅ | ✅ |
| `.xlsx` / `.xlsm` | ✅ | ✅ |
| `.xlsb` | ✅ | — |
| `.xls` | ✅ | — |
| `.ods` | ✅ | — |
| `.json` | ✅ | ✅ |
| `.md` (Markdown table) | ✅ | ✅ |
| `.html` (table) | ✅ | ✅ |

Reading preserves stored formulas where the source file provides them; XLSX
export writes a real, editable workbook with formulas kept live. Convert between
formats with `--output`:

```console
$ cargo run -p glasssheet-cli -- data.xlsx --output data.csv     # xlsx -> csv
$ cargo run -p glasssheet-cli -- budget.csv --output budget.xlsx # csv  -> xlsx
```

## Formula support

- **Operators:** `+ - * / ^`, comparison (`= <> < > <= >=`), text concat (`&`),
  unary minus, and postfix `%`. Exponentiation is right-associative.
- **References:** `A1`, absolute `$A$1`, ranges `A1:C4`, whole column/row
  (`A:A`, `1:1`), cross-sheet (`Sheet2!A1`), named ranges, and R1C1 style.
- **Arrays:** array literals `{1,2;3,4}` and dynamic spilling (with `#SPILL!`).
- **Functions** (~80), including:
  - Aggregation: `SUM`, `AVERAGE`, `MIN`, `MAX`, `COUNT(A)`, `PRODUCT`,
    `SUMIF(S)`, `COUNTIF(S)`, `AVERAGEIF(S)`, `SUMPRODUCT`, `SUBTOTAL`, `AGGREGATE`.
  - Math: `ABS`, `SQRT`, `POWER`, `MOD`, `INT`, `TRUNC`, `SIGN`, `ROUND(UP|DOWN)`,
    `MROUND`, `CEILING(.MATH)`, `FLOOR(.MATH)`, `EVEN`, `ODD`, `QUOTIENT`,
    `GCD`, `LCM`, `FACT`, `FACTDOUBLE`, `COMBIN(A)`, `PERMUT(ATIONA)`.
  - Trig/log: `PI`, `SIN…ATAN2`, `SINH…ATANH`, `DEGREES`, `RADIANS`, `LN`,
    `LOG`, `LOG10`, `EXP`, `SQRTPI`.
  - Stats/random: `MEDIAN`, `MODE`, `GEOMEAN`, `HARMEAN`, `TRIMMEAN`, `RAND`,
    `RANDBETWEEN`.
  - Numerals: `ROMAN`, `ARABIC`, `BASE`, `DECIMAL`.
  - Logical/text: `IF`, `AND`, `OR`, `NOT`, `CONCAT`/`CONCATENATE`, `LEN`,
    `UPPER`, `LOWER`, `TRIM`.
  - **GlassSheet extensions** (not in Excel): `COMPARE(a,b)` → −1/0/1,
    `SIMILARITY(t1,t2)` → 0–1 fuzzy match, and `AI(prompt)` (see below).
- **Error values** propagate through dependents: `#DIV/0!`, `#VALUE!`, `#REF!`,
  `#NAME?`, `#NUM!`, `#N/A`, `#SPILL!`, and `#CIRC!` for circular references.

## Beyond Excel

- **Incremental recalc** (`RecalcEngine`): editing one cell in a 10k-cell sheet
  recomputes in **~1.6 µs** (see `cargo bench`). Iterative mode handles
  intentional circular references.
- **Goal Seek:** `goal_seek` solves for the input that drives a formula to a
  target.
- **Rich cells:** styles, number formats, dates, conditional formatting, data
  validation, frozen panes, sheet protection, merged cells, undo/redo.
- **Threaded notes:** comments with replies, resolve state, tags, and
  timestamps — well past Excel's plain notes.
- **Themes:** Light / Dark / Glass palettes on the workbook.
- **AI functions:** `AI("…")` reads a refreshable, TTL-aware cache; a real
  model (hosted **or local**) plugs in as an `AiProvider` while the engine
  stays pure and offline. Stale results re-fetch on a background cadence.

## Using the engine as a library

```rust
use glasssheet_engine::{Sheet, CellRef, Value};

let mut sheet = Sheet::new("Sheet1");
sheet.set_input(CellRef::parse("A1").unwrap(), "10").unwrap();
sheet.set_input(CellRef::parse("A2").unwrap(), "20").unwrap();
sheet.set_formula(CellRef::parse("A3").unwrap(), "=A1+A2").unwrap();
assert_eq!(sheet.get(CellRef::parse("A3").unwrap()), Value::Number(30.0));
```

## Development

```console
$ cargo build
$ cargo test
$ cargo clippy --all-targets
$ cargo fmt
```

## Roadmap

Building on the same engine:

- ✅ Native `.xlsx` / `.xls` / `.xlsb` / `.ods` / `.csv` import; `.xlsx` / `.csv` export.
- Native desktop GUI (grid editor) with glassmorphism theming.
- ✅ Multi-sheet workbooks (`Workbook`) with cross-sheet references (`Sheet2!A1`).
- A web build (WASM) and a mobile app.
- A native GlassSheet document format.

## License

MIT — see [LICENSE](LICENSE).
