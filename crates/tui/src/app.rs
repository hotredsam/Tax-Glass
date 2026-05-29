//! Application state for the GlassSheet terminal UI.
//!
//! All behavior lives here as plain methods on [`App`] so it can be unit-tested
//! without a terminal. The render and event loop ([`crate::ui`], [`crate::run`])
//! are thin layers over this.

use glasssheet_engine::condformat;
use glasssheet_engine::{
    format_value, CellContent, CellRange, CellRef, CellStyle, Sheet, Theme, Value, Workbook,
};
use std::collections::HashMap;

/// Editing mode of the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Navigating the grid.
    Normal,
    /// Typing into the active cell.
    Edit,
    /// Typing a `:` command.
    Command,
}

/// Which screen the UI is showing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Screen {
    /// The cell grid.
    Grid,
    /// The "My Sheets" overview.
    Sheets,
    /// The file-open browser.
    Files,
}

/// An entry in the file browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileEntry {
    pub name: String,
    pub is_dir: bool,
}

/// File extensions the browser offers to open.
const SUPPORTED_EXTS: &[&str] = &[
    "csv", "tsv", "txt", "xlsx", "xlsm", "xlsb", "xls", "ods", "json", "md", "markdown", "html",
    "htm",
];

fn is_supported(name: &str) -> bool {
    name.rsplit_once('.')
        .map(|(_, ext)| SUPPORTED_EXTS.contains(&ext.to_ascii_lowercase().as_str()))
        .unwrap_or(false)
}

/// A single undoable cell edit.
struct Edit {
    sheet: usize,
    cell: (u32, u32),
    before: Option<CellContent>,
    after: Option<CellContent>,
}

/// The whole UI state.
pub struct App {
    pub wb: Workbook,
    /// Active cell `(col, row)`.
    pub cursor: (u32, u32),
    /// Selection anchor, set while extending a selection.
    pub anchor: Option<(u32, u32)>,
    /// Top-left `(col, row)` of the scrolled viewport.
    pub top: (u32, u32),
    pub mode: Mode,
    /// Edit/command text buffer.
    pub buffer: String,
    /// Transient status-line message.
    pub status: String,
    /// Animation tick (advances ~per frame).
    pub tick: u64,
    pub quit: bool,

    /// Which screen is visible.
    pub screen: Screen,
    /// Cursor on the "My Sheets" page.
    pub sheets_cursor: usize,
    /// Current directory shown in the file browser.
    pub cwd: std::path::PathBuf,
    /// Entries shown in the file browser.
    pub entries: Vec<FileEntry>,
    /// Cursor in the file browser.
    pub file_cursor: usize,

    undo: Vec<Edit>,
    redo: Vec<Edit>,
    computed: HashMap<(u32, u32), Value>,
    cond: HashMap<(u32, u32), CellStyle>,
}

impl Default for App {
    fn default() -> Self {
        App::new(Workbook::new())
    }
}

impl App {
    pub fn new(wb: Workbook) -> Self {
        let mut app = App {
            wb,
            cursor: (0, 0),
            anchor: None,
            top: (0, 0),
            mode: Mode::Normal,
            buffer: String::new(),
            status: "GlassSheet — arrows to move, Enter to edit, : for commands".into(),
            tick: 0,
            quit: false,
            screen: Screen::Grid,
            sheets_cursor: 0,
            cwd: std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from(".")),
            entries: Vec::new(),
            file_cursor: 0,
            undo: Vec::new(),
            redo: Vec::new(),
            computed: HashMap::new(),
            cond: HashMap::new(),
        };
        app.recompute();
        app
    }

    // --- access ---

    pub fn active(&self) -> &Sheet {
        self.wb
            .sheet_at(self.wb.active_index())
            .expect("active sheet")
    }

    fn active_mut(&mut self) -> &mut Sheet {
        let i = self.wb.active_index();
        self.wb.sheet_at_mut(i).expect("active sheet")
    }

    pub fn theme(&self) -> &Theme {
        self.wb.theme()
    }

    /// Re-evaluate the workbook and refresh the active sheet's value + style
    /// caches. Cross-sheet references resolve here.
    pub fn recompute(&mut self) {
        let name = self.active().name.clone();
        let mut all = self.wb.evaluate();
        self.computed = all.remove(&name).unwrap_or_default();
        self.cond = condformat::effective_styles(self.active().conditional_rules(), &self.computed);
    }

    /// Computed value at a cell on the active sheet.
    pub fn value_at(&self, col: u32, row: u32) -> Value {
        self.computed
            .get(&(col, row))
            .cloned()
            .unwrap_or(Value::Empty)
    }

    /// Display text for a cell: computed value through its number format.
    pub fn display_at(&self, col: u32, row: u32) -> String {
        let v = self.value_at(col, row);
        match self
            .active()
            .style(CellRef::new(col, row))
            .and_then(|s| s.number_format.as_deref())
        {
            Some(code) => format_value(&v, code),
            None => v.as_text(),
        }
    }

    /// Conditional-format style for a cell, if any rule matched.
    pub fn cond_style_at(&self, col: u32, row: u32) -> Option<&CellStyle> {
        self.cond.get(&(col, row))
    }

    /// Raw editable text for the active cell (formula with leading `=`).
    pub fn active_raw(&self) -> String {
        self.active()
            .raw_text(CellRef::new(self.cursor.0, self.cursor.1))
    }

    // --- navigation ---

    pub fn move_cursor(&mut self, dc: i64, dr: i64, extend: bool) {
        if extend {
            if self.anchor.is_none() {
                self.anchor = Some(self.cursor);
            }
        } else {
            self.anchor = None;
        }
        let col = (self.cursor.0 as i64 + dc).max(0) as u32;
        let row = (self.cursor.1 as i64 + dr).max(0) as u32;
        self.cursor = (col, row);
    }

    pub fn goto(&mut self, col: u32, row: u32) {
        self.anchor = None;
        self.cursor = (col, row);
    }

    /// Ensure the cursor is visible given a viewport of `cols` × `rows`.
    pub fn scroll_into_view(&mut self, cols: u32, rows: u32) {
        let (cc, cr) = self.cursor;
        if cc < self.top.0 {
            self.top.0 = cc;
        } else if cols > 0 && cc >= self.top.0 + cols {
            self.top.0 = cc + 1 - cols;
        }
        if cr < self.top.1 {
            self.top.1 = cr;
        } else if rows > 0 && cr >= self.top.1 + rows {
            self.top.1 = cr + 1 - rows;
        }
    }

    /// The current selection as a normalized range (cursor + anchor).
    pub fn selection(&self) -> CellRange {
        let (a, b) = (self.anchor.unwrap_or(self.cursor), self.cursor);
        CellRange::new(CellRef::new(a.0, a.1), CellRef::new(b.0, b.1))
    }

    /// Sum / count / average of numeric values in the selection.
    pub fn selection_stats(&self) -> (f64, usize, usize) {
        let mut sum = 0.0;
        let mut numeric = 0usize;
        let mut nonempty = 0usize;
        for cell in self.selection().cells() {
            let v = self.value_at(cell.col, cell.row);
            if v != Value::Empty {
                nonempty += 1;
            }
            if let Ok(n) = v.as_number() {
                if matches!(v, Value::Number(_)) {
                    sum += n;
                    numeric += 1;
                }
            }
        }
        (sum, numeric, nonempty)
    }

    // --- editing ---

    pub fn begin_edit(&mut self) {
        self.buffer = self.active_raw();
        self.mode = Mode::Edit;
    }

    pub fn begin_replace(&mut self) {
        self.buffer.clear();
        self.mode = Mode::Edit;
    }

    pub fn begin_command(&mut self) {
        self.buffer.clear();
        self.mode = Mode::Command;
    }

    pub fn push_char(&mut self, c: char) {
        self.buffer.push(c);
    }

    pub fn backspace(&mut self) {
        self.buffer.pop();
    }

    pub fn cancel(&mut self) {
        self.buffer.clear();
        self.mode = Mode::Normal;
    }

    /// Commit the edit buffer into the active cell, recording undo and moving
    /// down one row (spreadsheet Enter behavior).
    pub fn commit_edit(&mut self) {
        let text = std::mem::take(&mut self.buffer);
        let cur = self.cursor;
        self.apply_edit(cur, |sheet| {
            let _ = sheet.set_input(CellRef::new(cur.0, cur.1), &text);
        });
        self.mode = Mode::Normal;
        self.move_cursor(0, 1, false);
    }

    /// Clear the active cell (Delete).
    pub fn clear_cell(&mut self) {
        let cur = self.cursor;
        self.apply_edit(cur, |sheet| {
            sheet.set_value(CellRef::new(cur.0, cur.1), Value::Empty);
        });
    }

    /// Apply a mutation to one cell, capturing before/after for undo.
    fn apply_edit(&mut self, cell: (u32, u32), f: impl FnOnce(&mut Sheet)) {
        let sheet_idx = self.wb.active_index();
        let before = self.active().content(cell.0, cell.1).cloned();
        f(self.active_mut());
        let after = self.active().content(cell.0, cell.1).cloned();
        if before != after {
            self.undo.push(Edit {
                sheet: sheet_idx,
                cell,
                before,
                after,
            });
            self.redo.clear();
        }
        self.recompute();
    }

    pub fn undo(&mut self) {
        if let Some(edit) = self.undo.pop() {
            let _ = self.wb.set_active(edit.sheet);
            self.active_mut()
                .set_content(CellRef::new(edit.cell.0, edit.cell.1), edit.before.clone());
            self.cursor = edit.cell;
            self.redo.push(edit);
            self.recompute();
            self.status = "undo".into();
        } else {
            self.status = "nothing to undo".into();
        }
    }

    pub fn redo(&mut self) {
        if let Some(edit) = self.redo.pop() {
            let _ = self.wb.set_active(edit.sheet);
            self.active_mut()
                .set_content(CellRef::new(edit.cell.0, edit.cell.1), edit.after.clone());
            self.cursor = edit.cell;
            self.undo.push(edit);
            self.recompute();
            self.status = "redo".into();
        } else {
            self.status = "nothing to redo".into();
        }
    }

    // --- sheets ---

    pub fn next_sheet(&mut self) {
        let i = (self.wb.active_index() + 1) % self.wb.len().max(1);
        let _ = self.wb.set_active(i);
        self.goto(0, 0);
        self.recompute();
    }

    pub fn prev_sheet(&mut self) {
        let n = self.wb.len().max(1);
        let i = (self.wb.active_index() + n - 1) % n;
        let _ = self.wb.set_active(i);
        self.goto(0, 0);
        self.recompute();
    }

    // --- My Sheets overview ---

    /// Open the "My Sheets" page, pointing the cursor at the active sheet.
    pub fn open_sheets(&mut self) {
        self.sheets_cursor = self.wb.active_index();
        self.screen = Screen::Sheets;
    }

    /// Move the sheets cursor, wrapping around.
    pub fn sheets_move(&mut self, delta: i64) {
        let n = self.wb.len();
        if n == 0 {
            return;
        }
        self.sheets_cursor = (self.sheets_cursor as i64 + delta).rem_euclid(n as i64) as usize;
    }

    /// Open the sheet under the sheets cursor and return to the grid.
    pub fn sheets_select(&mut self) {
        if self.wb.set_active(self.sheets_cursor).is_ok() {
            self.goto(0, 0);
            self.recompute();
        }
        self.screen = Screen::Grid;
    }

    /// Per-sheet `(name, cols, rows, populated cells, active?)` for the page.
    pub fn sheet_summaries(&self) -> Vec<(String, u32, u32, usize, bool)> {
        let active = self.wb.active_index();
        self.wb
            .sheets()
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let (cols, rows) = s.dimensions();
                (s.name.clone(), cols, rows, s.len(), i == active)
            })
            .collect()
    }

    // --- file-open browser ---

    /// Open the file browser in the current working directory.
    pub fn open_files(&mut self) {
        let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
        self.browse_dir(cwd);
        self.screen = Screen::Files;
    }

    /// Point the browser at a directory and reload its entries.
    pub fn browse_dir(&mut self, dir: std::path::PathBuf) {
        self.cwd = dir;
        self.load_entries();
        self.file_cursor = 0;
    }

    fn load_entries(&mut self) {
        let mut entries = Vec::new();
        if self.cwd.parent().is_some() {
            entries.push(FileEntry {
                name: "..".into(),
                is_dir: true,
            });
        }
        match std::fs::read_dir(&self.cwd) {
            Ok(rd) => {
                let mut items: Vec<FileEntry> = rd
                    .filter_map(|e| e.ok())
                    .filter_map(|e| {
                        let name = e.file_name().to_string_lossy().to_string();
                        if name.starts_with('.') {
                            return None; // hide dotfiles
                        }
                        let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
                        if is_dir || is_supported(&name) {
                            Some(FileEntry { name, is_dir })
                        } else {
                            None
                        }
                    })
                    .collect();
                // Directories first, then files, each alphabetical.
                items.sort_by(|a, b| {
                    (!a.is_dir, a.name.to_lowercase()).cmp(&(!b.is_dir, b.name.to_lowercase()))
                });
                entries.extend(items);
            }
            Err(e) => self.status = format!("cannot read {}: {e}", self.cwd.display()),
        }
        self.entries = entries;
        if self.file_cursor >= self.entries.len() {
            self.file_cursor = self.entries.len().saturating_sub(1);
        }
    }

    /// Move the file-browser cursor, clamped.
    pub fn files_move(&mut self, delta: i64) {
        if self.entries.is_empty() {
            return;
        }
        let max = self.entries.len() as i64 - 1;
        self.file_cursor = (self.file_cursor as i64 + delta).clamp(0, max) as usize;
    }

    /// Act on the selected entry: descend into a directory, or open a file.
    pub fn files_enter(&mut self) {
        let Some(entry) = self.entries.get(self.file_cursor).cloned() else {
            return;
        };
        if entry.is_dir {
            let target = if entry.name == ".." {
                self.cwd.parent().map(|p| p.to_path_buf())
            } else {
                Some(self.cwd.join(&entry.name))
            };
            if let Some(dir) = target {
                self.browse_dir(dir);
            }
        } else {
            let path = self.cwd.join(&entry.name);
            self.open(&path.to_string_lossy());
            self.screen = Screen::Grid;
        }
    }

    /// Close any overlay screen and return to the grid.
    pub fn close_overlay(&mut self) {
        self.screen = Screen::Grid;
    }

    // --- commands (`:w file`, `:e file`, `:q`, `:sheet name`, `:theme name`) ---

    /// Run the command currently in the buffer; returns to Normal mode.
    pub fn run_command(&mut self) {
        let cmd = std::mem::take(&mut self.buffer);
        self.mode = Mode::Normal;
        let mut parts = cmd.trim().splitn(2, char::is_whitespace);
        let verb = parts.next().unwrap_or("");
        let arg = parts.next().unwrap_or("").trim();
        match verb {
            "q" | "quit" => self.quit = true,
            "w" | "write" | "save" => self.save(arg),
            "e" | "o" | "open" | "edit" => self.open(arg),
            "sheet" | "s" => self.select_sheet(arg),
            "sheets" | "ls" => self.open_sheets(),
            "files" | "browse" => self.open_files(),
            "new" => self.new_sheet(arg),
            "theme" => self.set_theme(arg),
            "" => {}
            other => self.status = format!("unknown command: {other}"),
        }
    }

    fn save(&mut self, path: &str) {
        if path.is_empty() {
            self.status = "usage: :w <file>".into();
            return;
        }
        match glasssheet_io::export_path(self.active(), path) {
            Ok(()) => self.status = format!("wrote {path}"),
            Err(e) => self.status = format!("write failed: {e}"),
        }
    }

    fn open(&mut self, path: &str) {
        if path.is_empty() {
            self.status = "usage: :e <file>".into();
            return;
        }
        match glasssheet_io::import_path(path) {
            Ok(sheet) => {
                let mut wb = Workbook::empty();
                let _ = wb.push_sheet(sheet);
                let theme = self.wb.theme().clone();
                wb.set_theme(theme);
                self.wb = wb;
                self.undo.clear();
                self.redo.clear();
                self.goto(0, 0);
                self.recompute();
                self.status = format!("opened {path}");
            }
            Err(e) => self.status = format!("open failed: {e}"),
        }
    }

    fn select_sheet(&mut self, name: &str) {
        match self.wb.index_of(name) {
            Some(i) => {
                let _ = self.wb.set_active(i);
                self.goto(0, 0);
                self.recompute();
                self.status = format!("sheet: {name}");
            }
            None => self.status = format!("no sheet named {name}"),
        }
    }

    fn new_sheet(&mut self, name: &str) {
        let name = if name.is_empty() {
            format!("Sheet{}", self.wb.len() + 1)
        } else {
            name.to_string()
        };
        match self.wb.add_sheet(&name) {
            Ok(i) => {
                let _ = self.wb.set_active(i);
                self.goto(0, 0);
                self.recompute();
                self.status = format!("added sheet {name}");
            }
            Err(e) => self.status = format!("{e}"),
        }
    }

    fn set_theme(&mut self, name: &str) {
        match Theme::builtin(name) {
            Some(t) => {
                self.wb.set_theme(t);
                self.status = format!("theme: {name}");
            }
            None => self.status = "themes: Light, Dark, Glass".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn edit_commit_recomputes_and_moves_down() {
        let mut app = App::default();
        app.begin_edit();
        app.buffer = "=2+3".into();
        app.commit_edit();
        // Value computed, cursor advanced to A2.
        assert_eq!(app.value_at(0, 0), Value::Number(5.0));
        assert_eq!(app.cursor, (0, 1));
        assert_eq!(app.mode, Mode::Normal);
    }

    #[test]
    fn undo_redo_roundtrip() {
        let mut app = App::default();
        app.goto(0, 0);
        app.begin_replace();
        app.buffer = "10".into();
        app.commit_edit();
        assert_eq!(app.value_at(0, 0), Value::Number(10.0));
        app.undo();
        assert_eq!(app.value_at(0, 0), Value::Empty);
        app.redo();
        assert_eq!(app.value_at(0, 0), Value::Number(10.0));
    }

    #[test]
    fn navigation_clamps_at_origin() {
        let mut app = App::default();
        app.move_cursor(-5, -5, false);
        assert_eq!(app.cursor, (0, 0));
        app.move_cursor(3, 2, false);
        assert_eq!(app.cursor, (3, 2));
    }

    #[test]
    fn scrolling_keeps_cursor_in_view() {
        let mut app = App::default();
        app.goto(20, 50);
        app.scroll_into_view(10, 20);
        assert!(app.cursor.0 < app.top.0 + 10 && app.cursor.0 >= app.top.0);
        assert!(app.cursor.1 < app.top.1 + 20 && app.cursor.1 >= app.top.1);
    }

    #[test]
    fn selection_stats_sum_numbers() {
        let mut app = App::default();
        for (r, v) in ["1", "2", "3"].iter().enumerate() {
            app.goto(0, r as u32);
            app.begin_replace();
            app.buffer = v.to_string();
            app.commit_edit();
        }
        // Select A1:A3.
        app.goto(0, 0);
        app.move_cursor(0, 2, true);
        let (sum, count, _) = app.selection_stats();
        assert_eq!((sum, count), (6.0, 3));
    }

    #[test]
    fn commands_add_and_switch_sheets() {
        let mut app = App::new(Workbook::new());
        app.buffer = "new Budget".into();
        app.run_command();
        assert_eq!(app.active().name, "Budget");
        app.buffer = "sheet Sheet1".into();
        app.run_command();
        assert_eq!(app.active().name, "Sheet1");
        app.buffer = "theme Glass".into();
        app.run_command();
        assert_eq!(app.theme().name, "Glass");
    }

    #[test]
    fn my_sheets_page_navigates_and_selects() {
        let mut app = App::new(Workbook::new());
        app.buffer = "new Budget".into();
        app.run_command(); // active is now Budget (index 1)
        app.open_sheets();
        assert_eq!(app.screen, Screen::Sheets);
        assert_eq!(app.sheets_cursor, 1);
        app.sheets_move(-1); // to Sheet1
        app.sheets_select();
        assert_eq!(app.screen, Screen::Grid);
        assert_eq!(app.active().name, "Sheet1");
        // Summaries report every sheet, flagging the active one.
        let summaries = app.sheet_summaries();
        assert_eq!(summaries.len(), 2);
        assert!(summaries
            .iter()
            .any(|(n, _, _, _, active)| n == "Sheet1" && *active));
    }

    #[test]
    fn file_browser_lists_and_opens() {
        // Build a temp dir with a CSV and a subdirectory.
        let dir = std::env::temp_dir().join(format!("glasssheet_fb_{}", std::process::id()));
        let _ = std::fs::create_dir_all(dir.join("sub"));
        std::fs::write(dir.join("data.csv"), "a,b\n1,2\n").unwrap();
        std::fs::write(dir.join("ignore.bin"), "x").unwrap();

        let mut app = App::new(Workbook::new());
        app.browse_dir(dir.clone());
        // ".." + "sub/" + "data.csv"; the unsupported .bin is filtered out.
        let names: Vec<&str> = app.entries.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&".."));
        assert!(names.contains(&"sub"));
        assert!(names.contains(&"data.csv"));
        assert!(!names.contains(&"ignore.bin"));

        // Move to the csv and open it.
        let idx = app
            .entries
            .iter()
            .position(|e| e.name == "data.csv")
            .unwrap();
        app.file_cursor = idx;
        app.files_enter();
        assert_eq!(app.screen, Screen::Grid);
        assert_eq!(app.value_at(0, 1), Value::Number(1.0)); // A2 == 1

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn cross_sheet_values_display() {
        let mut app = App::default();
        // Sheet1!A1 = 5
        app.begin_replace();
        app.buffer = "5".into();
        app.commit_edit();
        // Add Calc sheet referencing Sheet1.
        app.buffer = "new Calc".into();
        app.run_command();
        app.goto(0, 0);
        app.begin_replace();
        app.buffer = "=Sheet1!A1 * 2".into();
        app.commit_edit();
        assert_eq!(app.value_at(0, 0), Value::Number(10.0));
    }
}
