//! A1-style cell addressing: parsing and formatting references and ranges.
//!
//! Internally everything is zero-based `(col, row)`. The A1 surface syntax is
//! one-based and column-lettered (`A1`, `B2`, `AA10`), with optional `$`
//! absolute markers that are preserved on parse and re-emitted on format.

use crate::error::{EngineError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A single cell coordinate. Zero-based internally.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct CellRef {
    pub col: u32,
    pub row: u32,
    /// `$A1` — column fixed.
    pub col_abs: bool,
    /// `A$1` — row fixed.
    pub row_abs: bool,
}

impl CellRef {
    /// Construct from zero-based coordinates (relative).
    pub fn new(col: u32, row: u32) -> Self {
        CellRef {
            col,
            row,
            col_abs: false,
            row_abs: false,
        }
    }

    /// Parse an A1 reference like `B7` or `$AA$3`.
    pub fn parse(s: &str) -> Result<Self> {
        let s = s.trim();
        let bytes = s.as_bytes();
        let mut i = 0;

        let col_abs = bytes.get(i) == Some(&b'$');
        if col_abs {
            i += 1;
        }

        let letters_start = i;
        while i < bytes.len() && bytes[i].is_ascii_alphabetic() {
            i += 1;
        }
        if i == letters_start {
            return Err(EngineError::BadReference(s.to_string()));
        }
        let col = column_to_index(&s[letters_start..i])
            .ok_or_else(|| EngineError::BadReference(s.to_string()))?;

        let row_abs = bytes.get(i) == Some(&b'$');
        if row_abs {
            i += 1;
        }

        let digits = &s[i..];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(EngineError::BadReference(s.to_string()));
        }
        let row_one_based: u32 = digits
            .parse()
            .map_err(|_| EngineError::BadReference(s.to_string()))?;
        if row_one_based == 0 {
            return Err(EngineError::BadReference(s.to_string()));
        }

        Ok(CellRef {
            col,
            row: row_one_based - 1,
            col_abs,
            row_abs,
        })
    }

    /// Format as an A1 reference, honoring the absolute markers.
    pub fn to_a1(&self) -> String {
        let col_dollar = if self.col_abs { "$" } else { "" };
        let row_dollar = if self.row_abs { "$" } else { "" };
        format!(
            "{col_dollar}{}{row_dollar}{}",
            index_to_column(self.col),
            self.row + 1
        )
    }
}

impl fmt::Display for CellRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_a1())
    }
}

/// A rectangular range `A1:C4`. Stored normalized so `start <= end` on both axes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CellRange {
    pub start: CellRef,
    pub end: CellRef,
}

impl CellRange {
    /// Build a normalized range from two corners.
    pub fn new(a: CellRef, b: CellRef) -> Self {
        let start = CellRef {
            col: a.col.min(b.col),
            row: a.row.min(b.row),
            col_abs: a.col_abs,
            row_abs: a.row_abs,
        };
        let end = CellRef {
            col: a.col.max(b.col),
            row: a.row.max(b.row),
            col_abs: b.col_abs,
            row_abs: b.row_abs,
        };
        CellRange { start, end }
    }

    /// Parse `A1:C4`. A single reference without a colon becomes a 1×1 range.
    pub fn parse(s: &str) -> Result<Self> {
        match s.split_once(':') {
            Some((a, b)) => Ok(CellRange::new(CellRef::parse(a)?, CellRef::parse(b)?)),
            None => {
                let r = CellRef::parse(s)?;
                Ok(CellRange::new(r, r))
            }
        }
    }

    /// Iterate the cells of the range in row-major order.
    pub fn cells(&self) -> impl Iterator<Item = CellRef> + '_ {
        (self.start.row..=self.end.row).flat_map(move |row| {
            (self.start.col..=self.end.col).map(move |col| CellRef::new(col, row))
        })
    }

    /// Number of cells covered.
    pub fn len(&self) -> usize {
        let cols = (self.end.col - self.start.col + 1) as usize;
        let rows = (self.end.row - self.start.row + 1) as usize;
        cols * rows
    }

    /// Whether the range covers no cells. Always `false` for a valid range, but
    /// provided for API completeness / clippy.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl fmt::Display for CellRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", self.start.to_a1(), self.end.to_a1())
    }
}

/// Convert spreadsheet column letters (`A`, `Z`, `AA`) to a zero-based index.
/// Returns `None` on empty or non-alphabetic input.
pub fn column_to_index(letters: &str) -> Option<u32> {
    if letters.is_empty() {
        return None;
    }
    let mut idx: u32 = 0;
    for ch in letters.chars() {
        let c = ch.to_ascii_uppercase();
        if !c.is_ascii_uppercase() {
            return None;
        }
        idx = idx
            .checked_mul(26)?
            .checked_add((c as u32 - 'A' as u32) + 1)?;
    }
    Some(idx - 1)
}

/// Convert a zero-based column index to spreadsheet letters.
pub fn index_to_column(mut index: u32) -> String {
    let mut letters = Vec::new();
    loop {
        let rem = (index % 26) as u8;
        letters.push((b'A' + rem) as char);
        if index < 26 {
            break;
        }
        index = index / 26 - 1;
    }
    letters.iter().rev().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn column_letters_roundtrip() {
        for (letters, idx) in [
            ("A", 0),
            ("Z", 25),
            ("AA", 26),
            ("AZ", 51),
            ("BA", 52),
            ("ZZ", 701),
        ] {
            assert_eq!(column_to_index(letters), Some(idx), "{letters}");
            assert_eq!(index_to_column(idx), letters, "{idx}");
        }
    }

    #[test]
    fn parse_simple_reference() {
        let r = CellRef::parse("B3").unwrap();
        assert_eq!((r.col, r.row), (1, 2));
        assert!(!r.col_abs && !r.row_abs);
        assert_eq!(r.to_a1(), "B3");
    }

    #[test]
    fn parse_absolute_reference() {
        let r = CellRef::parse("$AA$10").unwrap();
        assert_eq!((r.col, r.row), (26, 9));
        assert!(r.col_abs && r.row_abs);
        assert_eq!(r.to_a1(), "$AA$10");
    }

    #[test]
    fn reject_bad_references() {
        for bad in ["", "1", "1A", "A", "A0", "$", "AB", "A1B"] {
            assert!(CellRef::parse(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn range_iterates_row_major() {
        let range = CellRange::parse("A1:B2").unwrap();
        let cells: Vec<_> = range.cells().map(|c| c.to_a1()).collect();
        assert_eq!(cells, ["A1", "B1", "A2", "B2"]);
        assert_eq!(range.len(), 4);
    }

    #[test]
    fn range_normalizes_reversed_corners() {
        let range = CellRange::parse("C4:A1").unwrap();
        assert_eq!(range.start.to_a1(), "A1");
        assert_eq!(range.end.col, 2);
    }

    #[test]
    fn single_ref_is_unit_range() {
        let range = CellRange::parse("D5").unwrap();
        assert_eq!(range.len(), 1);
    }
}
