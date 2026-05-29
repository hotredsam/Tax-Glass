//! Per-sheet view state: frozen panes, zoom, the active cell, and the current
//! selection. This is presentation/UI state — it never affects evaluation — but
//! it belongs to the document so it can be saved and restored.

use crate::address::{CellRange, CellRef};
use serde::{Deserialize, Serialize};

/// The saved view of a sheet.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ViewState {
    /// Number of rows frozen at the top (0 = none).
    pub frozen_rows: u32,
    /// Number of columns frozen at the left (0 = none).
    pub frozen_cols: u32,
    /// Zoom factor; `1.0` = 100%.
    pub zoom: f64,
    /// The active (focused) cell.
    pub active_cell: CellRef,
    /// The current selection, if any.
    pub selection: Option<CellRange>,
}

impl Default for ViewState {
    fn default() -> Self {
        ViewState {
            frozen_rows: 0,
            frozen_cols: 0,
            zoom: 1.0,
            active_cell: CellRef::new(0, 0),
            selection: None,
        }
    }
}

impl ViewState {
    /// Freeze `rows` rows at the top and `cols` columns at the left.
    pub fn freeze(&mut self, rows: u32, cols: u32) {
        self.frozen_rows = rows;
        self.frozen_cols = cols;
    }

    /// Remove all frozen panes.
    pub fn unfreeze(&mut self) {
        self.frozen_rows = 0;
        self.frozen_cols = 0;
    }

    /// Whether any pane is frozen.
    pub fn is_frozen(&self) -> bool {
        self.frozen_rows > 0 || self.frozen_cols > 0
    }

    /// Clamp the zoom to a sane range and set it.
    pub fn set_zoom(&mut self, zoom: f64) {
        self.zoom = zoom.clamp(0.1, 4.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sensible() {
        let v = ViewState::default();
        assert!(!v.is_frozen());
        assert_eq!(v.zoom, 1.0);
        assert_eq!(v.active_cell, CellRef::new(0, 0));
    }

    #[test]
    fn freeze_and_unfreeze() {
        let mut v = ViewState::default();
        v.freeze(1, 2);
        assert!(v.is_frozen());
        assert_eq!((v.frozen_rows, v.frozen_cols), (1, 2));
        v.unfreeze();
        assert!(!v.is_frozen());
    }

    #[test]
    fn zoom_is_clamped() {
        let mut v = ViewState::default();
        v.set_zoom(10.0);
        assert_eq!(v.zoom, 4.0);
        v.set_zoom(0.0);
        assert_eq!(v.zoom, 0.1);
    }
}
