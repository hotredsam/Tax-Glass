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

    /// Format in R1C1 notation relative to `base`. Absolute (`$`) parts render
    /// as fixed `R{n}`/`C{n}`; relative parts as `R[delta]`/`C[delta]` (or a
    /// bare `R`/`C` when the delta is zero).
    pub fn to_r1c1(&self, base: CellRef) -> String {
        let r = r1c1_part('R', self.row, self.row_abs, base.row);
        let c = r1c1_part('C', self.col, self.col_abs, base.col);
        format!("{r}{c}")
    }

    /// Format in absolute R1C1 notation (`R{row}C{col}`), ignoring `$` markers.
    pub fn to_r1c1_absolute(&self) -> String {
        format!("R{}C{}", self.row + 1, self.col + 1)
    }

    /// Parse an R1C1 reference relative to `base`. Handles absolute (`R1C1`),
    /// relative-with-delta (`R[-1]C[2]`), and bare relative (`RC`, `RC[1]`).
    pub fn parse_r1c1(s: &str, base: CellRef) -> Result<Self> {
        let s = s.trim();
        let bytes = s.as_bytes();
        let mut i = 0;

        let parse_part = |letter: u8, i: &mut usize, base_idx: u32| -> Result<(u32, bool)> {
            if bytes.get(*i) != Some(&letter) {
                return Err(EngineError::BadReference(s.to_string()));
            }
            *i += 1;
            // Bare letter (relative, delta 0): nothing or next is the other letter.
            let next = bytes.get(*i).copied();
            if next == Some(b'[') {
                *i += 1;
                let start = *i;
                if bytes.get(*i) == Some(&b'-') {
                    *i += 1;
                }
                while *i < bytes.len() && bytes[*i].is_ascii_digit() {
                    *i += 1;
                }
                let num: i64 = s[start..*i]
                    .parse()
                    .map_err(|_| EngineError::BadReference(s.to_string()))?;
                if bytes.get(*i) != Some(&b']') {
                    return Err(EngineError::BadReference(s.to_string()));
                }
                *i += 1;
                let idx = base_idx as i64 + num;
                if idx < 0 {
                    return Err(EngineError::BadReference(s.to_string()));
                }
                Ok((idx as u32, false))
            } else if matches!(next, Some(b'0'..=b'9')) {
                let start = *i;
                while *i < bytes.len() && bytes[*i].is_ascii_digit() {
                    *i += 1;
                }
                let num: u32 = s[start..*i]
                    .parse()
                    .map_err(|_| EngineError::BadReference(s.to_string()))?;
                if num == 0 {
                    return Err(EngineError::BadReference(s.to_string()));
                }
                Ok((num - 1, true))
            } else {
                // Bare relative, delta 0.
                Ok((base_idx, false))
            }
        };

        let (row, row_abs) = parse_part(b'R', &mut i, base.row)?;
        let (col, col_abs) = parse_part(b'C', &mut i, base.col)?;
        if i != s.len() {
            return Err(EngineError::BadReference(s.to_string()));
        }
        Ok(CellRef {
            col,
            row,
            col_abs,
            row_abs,
        })
    }
}

/// Render one R1C1 coordinate part.
fn r1c1_part(letter: char, idx: u32, abs: bool, base_idx: u32) -> String {
    if abs {
        format!("{letter}{}", idx + 1)
    } else {
        let delta = idx as i64 - base_idx as i64;
        if delta == 0 {
            letter.to_string()
        } else {
            format!("{letter}[{delta}]")
        }
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

    /// Whether a cell falls within this range (ignoring `$` markers).
    pub fn contains(&self, cell: CellRef) -> bool {
        cell.col >= self.start.col
            && cell.col <= self.end.col
            && cell.row >= self.start.row
            && cell.row <= self.end.row
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

    #[test]
    fn r1c1_absolute_and_relative_formatting() {
        let base = CellRef::parse("B2").unwrap(); // (col 1, row 1)
                                                  // Absolute $A$1 -> R1C1.
        assert_eq!(CellRef::parse("$A$1").unwrap().to_r1c1(base), "R1C1");
        // Relative A1 from B2: row delta -1, col delta -1.
        assert_eq!(CellRef::parse("A1").unwrap().to_r1c1(base), "R[-1]C[-1]");
        // Same cell as base -> RC.
        assert_eq!(CellRef::parse("B2").unwrap().to_r1c1(base), "RC");
        // Mixed: $A1 (col abs, row rel) from B2 -> R[-1]C1.
        assert_eq!(CellRef::parse("$A1").unwrap().to_r1c1(base), "R[-1]C1");
        assert_eq!(CellRef::parse("C5").unwrap().to_r1c1_absolute(), "R5C3");
    }

    #[test]
    fn r1c1_parse_roundtrips() {
        let base = CellRef::parse("B2").unwrap();
        for a1 in ["$A$1", "A1", "B2", "$A1", "A$1", "Z10"] {
            let r = CellRef::parse(a1).unwrap();
            let r1c1 = r.to_r1c1(base);
            let back = CellRef::parse_r1c1(&r1c1, base).unwrap();
            assert_eq!(back, r, "{a1} -> {r1c1}");
        }
    }

    #[test]
    fn r1c1_parse_specific_forms() {
        let base = CellRef::parse("C3").unwrap(); // (col 2, row 2)
        assert_eq!(
            CellRef::parse_r1c1("R1C1", base).unwrap(),
            CellRef::parse("$A$1").unwrap()
        );
        assert_eq!(
            CellRef::parse_r1c1("RC", base).unwrap(),
            CellRef::parse("C3").unwrap()
        );
        assert_eq!(
            CellRef::parse_r1c1("R[-1]C[1]", base).unwrap(),
            CellRef::parse("D2").unwrap()
        );
        assert!(CellRef::parse_r1c1("R[-5]C", base).is_err(), "negative row");
    }
}
