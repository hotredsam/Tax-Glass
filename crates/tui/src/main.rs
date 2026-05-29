//! `glasssheet-tui` — a full-screen terminal spreadsheet built on the GlassSheet
//! engine. Arrow keys move, typing edits, `:` runs commands (`:w file`,
//! `:e file`, `:new`, `:sheet`, `:theme`, `:q`).

mod app;
mod ui;

use app::{App, Mode};
use clap::Parser;
use crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use crossterm::ExecutableCommand;
use glasssheet_engine::Workbook;
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;
use std::io::{self, Stdout};
use std::path::PathBuf;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "glasssheet-tui",
    about = "A terminal spreadsheet (GlassSheet)",
    version
)]
struct Cli {
    /// Optional spreadsheet to open on launch (.csv/.xlsx/.xls/.ods/…).
    input: Option<PathBuf>,
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();
    let mut app = build_app(&cli);

    let mut terminal = setup_terminal()?;
    let result = run(&mut terminal, &mut app);
    restore_terminal(&mut terminal)?;
    result
}

fn build_app(cli: &Cli) -> App {
    if let Some(path) = &cli.input {
        match glasssheet_io::import_path(path) {
            Ok(sheet) => {
                let mut wb = Workbook::empty();
                let _ = wb.push_sheet(sheet);
                let mut app = App::new(wb);
                app.status = format!("opened {}", path.display());
                return app;
            }
            Err(e) => {
                let mut app = App::new(Workbook::new());
                app.status = format!("could not open {}: {e}", path.display());
                return app;
            }
        }
    }
    App::default()
}

fn setup_terminal() -> io::Result<Terminal<CrosstermBackend<Stdout>>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    stdout.execute(EnterAlternateScreen)?;
    Terminal::new(CrosstermBackend::new(stdout))
}

fn restore_terminal(terminal: &mut Terminal<CrosstermBackend<Stdout>>) -> io::Result<()> {
    disable_raw_mode()?;
    terminal.backend_mut().execute(LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    Ok(())
}

fn run(terminal: &mut Terminal<CrosstermBackend<Stdout>>, app: &mut App) -> io::Result<()> {
    // ~60 fps frame budget; the animated cursor/spinner advance every frame.
    let frame = Duration::from_millis(16);
    loop {
        terminal.draw(|f| ui::draw(f, app))?;
        if event::poll(frame)? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    handle_key(app, key.code, key.modifiers);
                }
            }
        }
        app.tick = app.tick.wrapping_add(1);
        if app.quit {
            return Ok(());
        }
    }
}

fn handle_key(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    match app.mode {
        Mode::Normal => handle_normal(app, code, mods),
        Mode::Edit => handle_edit(app, code),
        Mode::Command => handle_command(app, code),
    }
}

fn handle_normal(app: &mut App, code: KeyCode, mods: KeyModifiers) {
    let shift = mods.contains(KeyModifiers::SHIFT);
    let ctrl = mods.contains(KeyModifiers::CONTROL);
    match code {
        KeyCode::Char('c') | KeyCode::Char('q') if ctrl => app.quit = true,
        KeyCode::Char('z') if ctrl => app.undo(),
        KeyCode::Char('y') if ctrl => app.redo(),
        KeyCode::Up => app.move_cursor(0, -1, shift),
        KeyCode::Down => app.move_cursor(0, 1, shift),
        KeyCode::Left => app.move_cursor(-1, 0, shift),
        KeyCode::Right => app.move_cursor(1, 0, shift),
        KeyCode::PageUp => app.move_cursor(0, -10, shift),
        KeyCode::PageDown => app.move_cursor(0, 10, shift),
        KeyCode::Home => app.goto(0, app.cursor.1),
        KeyCode::Tab => app.next_sheet(),
        KeyCode::BackTab => app.prev_sheet(),
        KeyCode::Delete | KeyCode::Backspace => app.clear_cell(),
        KeyCode::Enter | KeyCode::F(2) => app.begin_edit(),
        KeyCode::Char(':') => app.begin_command(),
        KeyCode::Char(c) => {
            // Typing a character starts replacing the cell (Excel behavior).
            app.begin_replace();
            app.push_char(c);
        }
        KeyCode::Esc => app.anchor = None,
        _ => {}
    }
}

fn handle_edit(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.cancel(),
        KeyCode::Enter => app.commit_edit(),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Char(c) => app.push_char(c),
        _ => {}
    }
}

fn handle_command(app: &mut App, code: KeyCode) {
    match code {
        KeyCode::Esc => app.cancel(),
        KeyCode::Enter => app.run_command(),
        KeyCode::Backspace => app.backspace(),
        KeyCode::Char(c) => app.push_char(c),
        _ => {}
    }
}
