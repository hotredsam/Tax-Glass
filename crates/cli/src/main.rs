//! `glasssheet` — command-line front-end for the GlassSheet engine.
//!
//! Opens a spreadsheet in any supported format (`.csv`, `.tsv`, `.xlsx`,
//! `.xlsm`, `.xlsb`, `.xls`, `.ods`), evaluates its formulas, and either prints
//! the computed grid (table or CSV) or writes it back out via `--output`.

use clap::{Parser, ValueEnum};
use glasssheet_engine::{CellRef, Sheet, Value};
use glasssheet_io::{export_path, import_path};
use std::path::PathBuf;
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "glasssheet",
    about = "Open, evaluate, and convert spreadsheets (.csv/.xlsx/.xls/.xlsb/.ods)",
    version
)]
struct Cli {
    /// Input file. Any supported spreadsheet format; CSV cells starting with
    /// `=` are treated as formulas.
    input: PathBuf,

    /// Write the evaluated sheet to a file instead of stdout. Format is chosen
    /// by extension: `.csv`, `.tsv`, or `.xlsx` (formulas stay live in xlsx).
    #[arg(short, long)]
    output: Option<PathBuf>,

    /// Output format for stdout.
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
    let sheet = import_path(&cli.input)?;

    // Writing to a file takes precedence over stdout rendering.
    if let Some(out) = &cli.output {
        export_path(&sheet, out)?;
        eprintln!("wrote {}", out.display());
        return Ok(());
    }

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
