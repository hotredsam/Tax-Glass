//! Rendering for the terminal UI: a formula bar, the scrollable grid, a status
//! line, and sheet tabs — all painted in the workbook's theme colors with a
//! subtle animated cursor.

use crate::app::{App, Mode, Screen};
use glasssheet_engine::address::index_to_column;
use glasssheet_engine::style::Color as GColor;
use glasssheet_engine::Value;
use ratatui::prelude::*;
use ratatui::widgets::Paragraph;

const ROW_HDR_W: u16 = 5;
const CELL_W: u16 = 10;

fn col(c: GColor) -> Color {
    Color::Rgb(c.r, c.g, c.b)
}

fn lerp(a: u8, b: u8, t: f64) -> u8 {
    (a as f64 + (b as f64 - a as f64) * t)
        .round()
        .clamp(0.0, 255.0) as u8
}

fn blend(a: GColor, b: GColor, t: f64) -> Color {
    Color::Rgb(lerp(a.r, b.r, t), lerp(a.g, b.g, t), lerp(a.b, b.b, t))
}

/// Draw the whole UI for one frame, dispatching on the active screen.
pub fn draw(f: &mut Frame, app: &mut App) {
    match app.screen {
        Screen::Grid => draw_grid_screen(f, app),
        Screen::Sheets => draw_sheets(f, app),
        Screen::Files => draw_files(f, app),
    }
}

fn draw_grid_screen(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let rows = Layout::vertical([
        Constraint::Length(1), // formula bar
        Constraint::Min(1),    // grid
        Constraint::Length(1), // status
        Constraint::Length(1), // tabs
    ])
    .split(area);

    draw_formula_bar(f, app, rows[0]);
    draw_grid(f, app, rows[1]);
    draw_status(f, app, rows[2]);
    draw_tabs(f, app, rows[3]);
}

/// A generic full-screen list overlay (title, highlighted rows, footer hint).
fn draw_list_overlay(
    f: &mut Frame,
    app: &App,
    title: &str,
    items: Vec<(String, bool)>,
    hint: &str,
) {
    let p = app.theme().palette.clone();
    let area = f.area();
    let rows = Layout::vertical([
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
    ])
    .split(area);

    let accent = *p.accents.first().unwrap_or(&p.selection);
    f.render_widget(
        Paragraph::new(format!(" {title}")).style(
            Style::default()
                .bg(col(accent))
                .fg(col(p.background))
                .add_modifier(Modifier::BOLD),
        ),
        rows[0],
    );

    let lines: Vec<Line> = items
        .into_iter()
        .map(|(text, selected)| {
            let style = if selected {
                Style::default()
                    .bg(col(p.selection))
                    .fg(col(p.foreground))
                    .add_modifier(Modifier::BOLD)
            } else {
                Style::default().bg(col(p.background)).fg(col(p.foreground))
            };
            Line::from(Span::styled(format!(" {text}"), style))
        })
        .collect();
    f.render_widget(
        Paragraph::new(lines).style(Style::default().bg(col(p.background))),
        rows[1],
    );

    f.render_widget(
        Paragraph::new(format!(" {hint}"))
            .style(Style::default().bg(col(p.header)).fg(col(p.foreground))),
        rows[2],
    );
}

fn draw_sheets(f: &mut Frame, app: &App) {
    let items: Vec<(String, bool)> = app
        .sheet_summaries()
        .into_iter()
        .enumerate()
        .map(|(i, (name, cols, rows, cells, active))| {
            let marker = if active { "●" } else { " " };
            (
                format!("{marker} {name:<24} {cols}×{rows}, {cells} cells"),
                i == app.sheets_cursor,
            )
        })
        .collect();
    draw_list_overlay(
        f,
        app,
        "My Sheets",
        items,
        "↑/↓ select   Enter open   Esc back",
    );
}

fn draw_files(f: &mut Frame, app: &App) {
    let items: Vec<(String, bool)> = app
        .entries
        .iter()
        .enumerate()
        .map(|(i, e)| {
            let label = if e.is_dir {
                format!("[dir]  {}/", e.name)
            } else {
                format!("       {}", e.name)
            };
            (label, i == app.file_cursor)
        })
        .collect();
    let title = format!("Open File — {}", app.cwd.display());
    draw_list_overlay(
        f,
        app,
        &title,
        items,
        "↑/↓ select   Enter open/descend   Esc cancel",
    );
}

fn draw_formula_bar(f: &mut Frame, app: &App, area: Rect) {
    let p = app.theme().palette.clone();
    let addr = cell_name(app.cursor);
    let content = match app.mode {
        Mode::Command => format!(":{}", app.buffer),
        Mode::Edit => format!("{addr}  {}", app.buffer),
        Mode::Normal => format!("{addr}  {}", app.active_raw()),
    };
    let style = Style::default().fg(col(p.foreground)).bg(col(p.header));
    f.render_widget(Paragraph::new(content).style(style), area);
}

fn draw_grid(f: &mut Frame, app: &mut App, area: Rect) {
    let p = app.theme().palette.clone();
    let visible_cols = ((area.width.saturating_sub(ROW_HDR_W)) / CELL_W).max(1) as u32;
    let visible_rows = area.height.saturating_sub(1).max(1) as u32; // minus header
    app.scroll_into_view(visible_cols, visible_rows);

    let cursor = app.cursor;
    let sel = app.selection();
    // Animated cursor pulse between selection and the first accent color.
    let t = 0.5 + 0.5 * (app.tick as f64 * 0.25).sin();
    let accent = *app.theme().palette.accents.first().unwrap_or(&p.selection);

    let mut lines: Vec<Line> = Vec::with_capacity(visible_rows as usize + 1);

    // Column header row.
    let mut header = vec![Span::styled(
        format!("{:>width$} ", "", width = (ROW_HDR_W - 1) as usize),
        Style::default().bg(col(p.header)).fg(col(p.foreground)),
    )];
    for vc in 0..visible_cols {
        let c = app.top.0 + vc;
        let label = index_to_column(c);
        let active = c == cursor.0;
        let style = Style::default()
            .bg(col(p.header))
            .fg(col(p.foreground))
            .add_modifier(if active {
                Modifier::BOLD
            } else {
                Modifier::empty()
            });
        header.push(Span::styled(center(&label, CELL_W as usize), style));
    }
    lines.push(Line::from(header));

    // Data rows.
    for vr in 0..visible_rows {
        let r = app.top.1 + vr;
        let mut spans = vec![Span::styled(
            format!("{:>width$} ", r + 1, width = (ROW_HDR_W - 1) as usize),
            Style::default()
                .bg(col(p.header))
                .fg(col(p.foreground))
                .add_modifier(if r == cursor.1 {
                    Modifier::BOLD
                } else {
                    Modifier::empty()
                }),
        )];
        for vc in 0..visible_cols {
            let c = app.top.0 + vc;
            let value = app.value_at(c, r);
            let text = pad_cell(&app.display_at(c, r), &value);

            let mut style = Style::default().fg(col(p.foreground)).bg(col(p.background));
            // Conditional formatting fill/text.
            if let Some(cs) = app.cond_style_at(c, r) {
                if let Some(fill) = cs.fill {
                    style = style.bg(col(fill));
                }
                if let Some(fg) = cs.font.color {
                    style = style.fg(col(fg));
                }
            }
            if value.is_error() {
                style = style.fg(Color::Rgb(0xFF, 0x6B, 0x6B));
            }
            // Selection + animated cursor highlight.
            let in_sel = sel.contains(glasssheet_engine::CellRef::new(c, r));
            if (c, r) == cursor {
                style = style
                    .bg(blend(p.selection, accent, t))
                    .add_modifier(Modifier::BOLD);
            } else if in_sel {
                style = style.bg(col(p.selection));
            }
            spans.push(Span::styled(text, style));
        }
        lines.push(Line::from(spans));
    }

    f.render_widget(Paragraph::new(lines), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let p = app.theme().palette.clone();
    let mode = match app.mode {
        Mode::Normal => "NORMAL",
        Mode::Edit => "EDIT",
        Mode::Command => "COMMAND",
    };
    let (sum, count, nonempty) = app.selection_stats();
    let stats = if count > 0 {
        format!(
            "  Sum {} | Avg {} | Count {}",
            trim(sum),
            trim(sum / count as f64),
            count
        )
    } else if nonempty > 0 {
        format!("  Count {nonempty}")
    } else {
        String::new()
    };
    let spinner = ['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
    let tick_mark = spinner[(app.tick as usize) % spinner.len()];
    let text = format!(" {tick_mark} {mode}  {}{}", app.status, stats);
    let style = Style::default()
        .bg(blend(
            p.header,
            *p.accents.first().unwrap_or(&p.header),
            0.15,
        ))
        .fg(col(p.foreground));
    f.render_widget(Paragraph::new(text).style(style), area);
}

fn draw_tabs(f: &mut Frame, app: &App, area: Rect) {
    let p = app.theme().palette.clone();
    let active = app.wb.active_index();
    let mut spans = Vec::new();
    for (i, name) in app.wb.sheet_names().iter().enumerate() {
        let style = if i == active {
            Style::default()
                .bg(col(*p.accents.first().unwrap_or(&p.selection)))
                .fg(col(p.background))
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default().bg(col(p.header)).fg(col(p.foreground))
        };
        spans.push(Span::styled(format!(" {name} "), style));
        spans.push(Span::raw(" "));
    }
    f.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(col(p.background))),
        area,
    );
}

// --- helpers ---

fn cell_name((c, r): (u32, u32)) -> String {
    format!("{}{}", index_to_column(c), r + 1)
}

fn center(s: &str, width: usize) -> String {
    if s.len() >= width {
        return s.chars().take(width).collect();
    }
    let total = width - s.len();
    let left = total / 2;
    format!("{}{}{}", " ".repeat(left), s, " ".repeat(total - left))
}

/// Pad/truncate a cell's display text to the column width, right-aligning
/// numbers and left-aligning everything else.
fn pad_cell(text: &str, value: &Value) -> String {
    let w = CELL_W as usize;
    let mut t: String = text.chars().take(w - 1).collect();
    let pad = w - 1 - t.chars().count();
    if matches!(value, Value::Number(_)) {
        t = format!("{}{}", " ".repeat(pad), t);
    } else {
        t.push_str(&" ".repeat(pad));
    }
    format!("{t} ")
}

fn trim(n: f64) -> String {
    glasssheet_engine::value::format_number(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use glasssheet_engine::Workbook;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    /// Render a frame into an in-memory backend and confirm the grid paints
    /// (column header `A` and an edited value both appear). Proves the full
    /// draw path works without a real terminal.
    #[test]
    fn renders_grid_headlessly() {
        let mut app = App::new(Workbook::new());
        app.begin_replace();
        app.push_char('4');
        app.push_char('2');
        app.commit_edit(); // A1 = 42

        let mut term = Terminal::new(TestBackend::new(60, 16)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();

        let content: String = term
            .backend()
            .buffer()
            .content()
            .iter()
            .map(|c| c.symbol())
            .collect();
        assert!(content.contains('A'), "column header A should render");
        assert!(content.contains("42"), "edited value should render");
        assert!(content.contains("Sheet1"), "sheet tab should render");
    }

    /// Not a real test — render a populated sheet and print it, so the layout
    /// can be eyeballed via `cargo test -- --ignored --nocapture`.
    #[test]
    #[ignore]
    fn dump_preview() {
        let mut app = App::new(Workbook::new());
        let rows = [
            ("Item", "Qty", "Price", ""),
            ("Widgets", "4", "2.5", "=B2*C2"),
            ("Gadgets", "3", "9.99", "=B3*C3"),
            ("Gizmos", "10", "1.25", "=B4*C4"),
        ];
        for (r, (a, b, c, d)) in rows.iter().enumerate() {
            for (col, txt) in [a, b, c, d].iter().enumerate() {
                if txt.is_empty() {
                    continue;
                }
                app.goto(col as u32, r as u32);
                app.begin_replace();
                for ch in txt.chars() {
                    app.push_char(ch);
                }
                app.commit_edit();
            }
        }
        app.goto(3, 4);
        app.begin_replace();
        for ch in "=SUM(D2:D4)".chars() {
            app.push_char(ch);
        }
        app.commit_edit();
        app.goto(3, 1);

        let (w, h) = (62u16, 12u16);
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, &mut app)).unwrap();
        let buf = term.backend().buffer().clone();
        println!("\n+{}+", "-".repeat(w as usize));
        for y in 0..h {
            let mut line = String::new();
            for x in 0..w {
                line.push_str(buf[(x, y)].symbol());
            }
            println!("|{line}|");
        }
        println!("+{}+", "-".repeat(w as usize));
    }
}
