//! `glasssheet` — command-line front-end for the GlassSheet engine.
//!
//! Loads a CSV where any cell may contain a formula (a leading `=`), evaluates
//! it, and prints the computed grid as an aligned table or back out as CSV.

use clap::{Parser, ValueEnum};
use glasssheet_engine::{CellRef, Sheet, Value};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "glasssheet",
    about = "Evaluate a CSV spreadsheet with formulas (e.g. =SUM(A1:A3))",
    version
)]
struct Cli {
    /// Input CSV file. Cells starting with `=` are treated as formulas.
    input: PathBuf,

    /// Output format.
    #[arg(short, long, value_enum, default_value_t = Format::Table)]
    format: Format,

    /// Print only the computed value of a single cell (e.g. `--cell B4`).
    #[arg(short, long)]
    cell: Option<String>,

    /// Show raw cell text (formulas) instead of computed values.
    #[arg(long)]
    raw: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum Format {
    Table,
    Csv,
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(&cli) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: &Cli) -> Result<(), Box<dyn std::error::Error>> {
    let sheet = load_csv(&cli.input)?;

    if let Some(addr) = &cli.cell {
        let r = CellRef::parse(addr)?;
        let text = if cli.raw {
            sheet.raw_text(r)
        } else {
            sheet.get(r).as_text()
        };
        println!("{text}");
        return Ok(());
    }

    let grid = build_grid(&sheet, cli.raw);
    match cli.format {
        Format::Csv => print_csv(&grid)?,
        Format::Table => print_table(&grid),
    }
    Ok(())
}

/// Read a CSV into a sheet, inferring cell types via `set_input`.
fn load_csv(path: &PathBuf) -> Result<Sheet, Box<dyn std::error::Error>> {
    let mut reader = csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .from_path(path)?;

    let mut sheet = Sheet::new("Sheet1");
    for (row_idx, record) in reader.records().enumerate() {
        let record = record?;
        for (col_idx, field) in record.iter().enumerate() {
            if field.is_empty() {
                continue;
            }
            let r = CellRef::new(col_idx as u32, row_idx as u32);
            sheet.set_input(r, field)?;
        }
    }
    Ok(sheet)
}

/// Materialize the sheet into a dense `rows × cols` grid of strings.
fn build_grid(sheet: &Sheet, raw: bool) -> Vec<Vec<String>> {
    let (cols, rows) = sheet.dimensions();
    let computed = sheet.evaluate();
    let mut grid = Vec::with_capacity(rows as usize);
    for row in 0..rows {
        let mut line = Vec::with_capacity(cols as usize);
        for col in 0..cols {
            let text = if raw {
                sheet.raw_text(CellRef::new(col, row))
            } else {
                computed
                    .get(&(col, row))
                    .cloned()
                    .unwrap_or(Value::Empty)
                    .as_text()
            };
            line.push(text);
        }
        grid.push(line);
    }
    grid
}

fn print_csv(grid: &[Vec<String>]) -> Result<(), Box<dyn std::error::Error>> {
    let mut writer = csv::Writer::from_writer(std::io::stdout());
    for row in grid {
        writer.write_record(row)?;
    }
    writer.flush()?;
    Ok(())
}

/// Print an aligned table with column-letter and row-number headers.
fn print_table(grid: &[Vec<String>]) {
    let cols = grid.iter().map(|r| r.len()).max().unwrap_or(0);
    if cols == 0 {
        println!("(empty sheet)");
        return;
    }
    let row_label_w = grid.len().to_string().len().max(1);

    // Column widths account for the A/B/C header and every cell.
    let mut widths = vec![0usize; cols];
    for (c, w) in widths.iter_mut().enumerate() {
        *w = glasssheet_engine::address::index_to_column(c as u32).len();
    }
    for row in grid {
        for (c, cell) in row.iter().enumerate() {
            widths[c] = widths[c].max(cell.chars().count());
        }
    }

    // Header row.
    print!("{:>width$} ", "", width = row_label_w);
    for (c, w) in widths.iter().enumerate() {
        print!(
            "| {:^width$} ",
            glasssheet_engine::address::index_to_column(c as u32),
            width = *w
        );
    }
    println!();

    // Separator.
    print!("{}-", "-".repeat(row_label_w));
    for w in &widths {
        print!("+{}", "-".repeat(w + 2));
    }
    println!();

    // Data rows.
    for (r, row) in grid.iter().enumerate() {
        print!("{:>width$} ", r + 1, width = row_label_w);
        for (c, w) in widths.iter().enumerate() {
            let cell = row.get(c).map(String::as_str).unwrap_or("");
            print!("| {:>width$} ", cell, width = *w);
        }
        println!();
    }
}
