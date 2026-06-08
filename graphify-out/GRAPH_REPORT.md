# Graph Report - .  (2026-06-07)

## Corpus Check
- Corpus is ~26,237 words - fits in a single context window. You may not need a graph.

## Summary
- 692 nodes · 1697 edges · 26 communities (24 shown, 2 thin omitted)
- Extraction: 99% EXTRACTED · 1% INFERRED · 0% AMBIGUOUS · INFERRED: 9 edges (avg confidence: 0.8)
- Token cost: 0 input · 0 output

## Community Hubs (Navigation)
- [[_COMMUNITY_Engine Core Types|Engine Core Types]]
- [[_COMMUNITY_Formula Expression AST|Formula Expression AST]]
- [[_COMMUNITY_Formula Parser|Formula Parser]]
- [[_COMMUNITY_Sheet Grid Model|Sheet Grid Model]]
- [[_COMMUNITY_Cell Address A1|Cell Address A1]]
- [[_COMMUNITY_Cell Dependencies|Cell Dependencies]]
- [[_COMMUNITY_Sheet File IO|Sheet File IO]]
- [[_COMMUNITY_Recalculation Engine|Recalculation Engine]]
- [[_COMMUNITY_Undo History|Undo History]]
- [[_COMMUNITY_Number Formatting|Number Formatting]]
- [[_COMMUNITY_Date Time Serial|Date Time Serial]]
- [[_COMMUNITY_Cell Style|Cell Style]]
- [[_COMMUNITY_Cell Validation|Cell Validation]]
- [[_COMMUNITY_Value Coercion Errors|Value Coercion Errors]]
- [[_COMMUNITY_CLI Main Entry|CLI Main Entry]]
- [[_COMMUNITY_View Freeze State|View Freeze State]]
- [[_COMMUNITY_Conditional Formatting|Conditional Formatting]]
- [[_COMMUNITY_Agent Swarm State|Agent Swarm State]]
- [[_COMMUNITY_Agent Config|Agent Config]]
- [[_COMMUNITY_Session Current State|Session Current State]]
- [[_COMMUNITY_Formula Property Tests|Formula Property Tests]]
- [[_COMMUNITY_IO Error Conversion|IO Error Conversion]]
- [[_COMMUNITY_Ranked Context Cache|Ranked Context Cache]]
- [[_COMMUNITY_Permission Settings|Permission Settings]]
- [[_COMMUNITY_Engine Error|Engine Error]]

## God Nodes (most connected - your core abstractions)
1. `Sheet` - 63 edges
2. `Workbook` - 35 edges
3. `Value` - 33 edges
4. `CellRef` - 29 edges
5. `Evaluator<'a>` - 25 edges
6. `cell()` - 24 edges
7. `Expr` - 21 edges
8. `Expr` - 20 edges
9. `sheet_with()` - 20 edges
10. `Parser` - 19 edges

## Surprising Connections (you probably didn't know these)
- `run()` --calls--> `export_path()`  [INFERRED]
  crates/cli/src/main.rs → crates/io/src/export.rs
- `run()` --calls--> `import_path()`  [INFERRED]
  crates/cli/src/main.rs → crates/io/src/import.rs
- `csv_roundtrip_writes_computed_values()` --calls--> `export_path()`  [INFERRED]
  crates/io/tests/roundtrip.rs → crates/io/src/export.rs
- `unsupported_extension_is_rejected()` --calls--> `export_path()`  [INFERRED]
  crates/io/tests/roundtrip.rs → crates/io/src/export.rs
- `xlsx_roundtrip_preserves_values_and_formulas()` --calls--> `export_path()`  [INFERRED]
  crates/io/tests/roundtrip.rs → crates/io/src/export.rs

## Import Cycles
- 1-file cycle: `crates/cli/src/main.rs -> crates/cli/src/main.rs`
- 1-file cycle: `crates/engine/src/condformat.rs -> crates/engine/src/condformat.rs`
- 1-file cycle: `crates/engine/src/deps.rs -> crates/engine/src/deps.rs`
- 1-file cycle: `crates/engine/src/eval.rs -> crates/engine/src/eval.rs`
- 1-file cycle: `crates/engine/src/format.rs -> crates/engine/src/format.rs`
- 1-file cycle: `crates/engine/src/recalc.rs -> crates/engine/src/recalc.rs`
- 1-file cycle: `crates/engine/src/sheet.rs -> crates/engine/src/sheet.rs`
- 1-file cycle: `crates/engine/src/undo.rs -> crates/engine/src/undo.rs`
- 1-file cycle: `crates/engine/src/validation.rs -> crates/engine/src/validation.rs`
- 1-file cycle: `crates/engine/src/workbook.rs -> crates/engine/src/workbook.rs`
- 1-file cycle: `crates/io/src/error.rs -> crates/io/src/error.rs`
- 1-file cycle: `crates/io/src/export.rs -> crates/io/src/export.rs`
- 1-file cycle: `crates/io/src/import.rs -> crates/io/src/import.rs`

## Communities (26 total, 2 thin omitted)

### Community 0 - "Engine Core Types"
Cohesion: 0.06
Nodes (56): CellRange, CellRef, CellStyle, Expr, HashMap, HashSet, Into, Item (+48 more)

### Community 1 - "Formula Expression AST"
Cohesion: 0.07
Nodes (63): BinOp, CellContent, CellRange, CellRef, Expr, Fn, HashMap, HashSet (+55 more)

### Community 2 - "Formula Parser"
Cohesion: 0.11
Nodes (41): CellRange, CellRef, Option, Result, String, Vec, BinOp, binop_str() (+33 more)

### Community 3 - "Sheet Grid Model"
Cohesion: 0.10
Nodes (26): CellRange, Default, HashMap, Into, Item, Iterator, Option, Result (+18 more)

### Community 4 - "Cell Address A1"
Cohesion: 0.11
Nodes (21): Display, Formatter, Item, Iterator, Option, Result, Self, String (+13 more)

### Community 5 - "Cell Dependencies"
Cohesion: 0.18
Nodes (23): CellContent, CellKey, CellRef, Expr, HashMap, HashSet, Item, Iterator (+15 more)

### Community 6 - "Sheet File IO"
Cohesion: 0.14
Nodes (27): AsRef, Path, Result, Sheet, String, AsRef, Option, Path (+19 more)

### Community 7 - "Recalculation Engine"
Cohesion: 0.17
Nodes (18): CellKey, CellRef, HashMap, HashSet, Into, Result, Self, Sheet (+10 more)

### Community 8 - "Undo History"
Cohesion: 0.17
Nodes (16): CellContent, CellRef, Option, Result, Self, Sheet, Value, Vec (+8 more)

### Community 9 - "Number Formatting"
Cohesion: 0.16
Nodes (15): CellError, String, Value, Vec, apply_text_section(), fmt(), format_number_section(), format_number_value() (+7 more)

### Community 10 - "Date Time Serial"
Cohesion: 0.20
Nodes (20): String, Vec, civil_from_days(), date_to_serial(), DateTime, datetime_to_serial(), days_from_civil(), dt() (+12 more)

### Community 11 - "Cell Style"
Cohesion: 0.19
Nodes (13): Option, Self, String, Alignment, Border, Borders, BorderStyle, CellStyle (+5 more)

### Community 12 - "Cell Validation"
Cohesion: 0.17
Nodes (12): CellRange, Fn, Into, Option, Self, String, Value, Vec (+4 more)

### Community 13 - "Value Coercion Errors"
Cohesion: 0.18
Nodes (7): Display, Formatter, Result, String, CellError, format_number(), Value

### Community 14 - "CLI Main Entry"
Cohesion: 0.21
Nodes (17): Box, Error, Option, Result, Sheet, String, Vec, ExitCode (+9 more)

### Community 15 - "View Freeze State"
Cohesion: 0.19
Nodes (9): CellRange, CellRef, Default, Option, Self, defaults_are_sensible(), freeze_and_unfreeze(), ViewState (+1 more)

### Community 16 - "Conditional Formatting"
Cohesion: 0.31
Nodes (9): CellRange, CellStyle, HashMap, Self, Value, Condition, effective_styles(), first_matching_rule_wins() (+1 more)

### Community 17 - "Agent Swarm State"
Cohesion: 0.14
Nodes (13): consensus, history, pending, createdAt, initialized, queen, agentId, electedAt (+5 more)

### Community 18 - "Agent Config"
Cohesion: 0.17
Nodes (11): features, agentTeams, autoMemory, hooks, statusLine, memory, importOnSessionStart, storePath (+3 more)

### Community 19 - "Session Current State"
Cohesion: 0.17
Nodes (11): context, cwd, id, metrics, commands, edits, errors, tasks (+3 more)

### Community 20 - "Formula Property Tests"
Cohesion: 0.33
Nodes (4): String, Value, Strategy, arb_formula()

### Community 21 - "IO Error Conversion"
Cohesion: 0.47
Nodes (4): Error, Self, From, IoError

### Community 22 - "Ranked Context Cache"
Cohesion: 0.50
Nodes (3): computedAt, entries, version

## Knowledge Gaps
- **89 isolated node(s):** `schemaVersion`, `profile`, `hooks`, `statusLine`, `autoMemory` (+84 more)
  These have ≤1 connection - possible missing edges or undocumented components.
- **2 thin communities (<3 nodes) omitted from report** — run `graphify query` to explore isolated nodes.

## Suggested Questions
_Questions this graph is uniquely positioned to answer:_

- **Why does `CellError` connect `Number Formatting` to `Engine Core Types`, `Formula Expression AST`, `Recalculation Engine`?**
  _High betweenness centrality (0.049) - this node is a cross-community bridge._
- **Why does `evaluate_sheet()` connect `Formula Expression AST` to `Engine Core Types`?**
  _High betweenness centrality (0.034) - this node is a cross-community bridge._
- **What connects `schemaVersion`, `profile`, `hooks` to the rest of the system?**
  _89 weakly-connected nodes found - possible documentation gaps or missing edges._
- **Should `Engine Core Types` be split into smaller, more focused modules?**
  _Cohesion score 0.05871943371943372 - nodes in this community are weakly interconnected._
- **Should `Formula Expression AST` be split into smaller, more focused modules?**
  _Cohesion score 0.06513157894736842 - nodes in this community are weakly interconnected._
- **Should `Formula Parser` be split into smaller, more focused modules?**
  _Cohesion score 0.10765027322404372 - nodes in this community are weakly interconnected._
- **Should `Sheet Grid Model` be split into smaller, more focused modules?**
  _Cohesion score 0.10033670033670034 - nodes in this community are weakly interconnected._