//! Cell styling model: fonts, fills, borders, alignment, and a per-cell number
//! format. Styles are presentation only — they never affect evaluation — and
//! travel with cells across structural edits.

use serde::{Deserialize, Serialize};

/// An RGB color.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const BLACK: Color = Color { r: 0, g: 0, b: 0 };
    pub const WHITE: Color = Color {
        r: 255,
        g: 255,
        b: 255,
    };

    pub fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Parse a `#RRGGBB` (or `RRGGBB`) hex string.
    pub fn from_hex(s: &str) -> Option<Self> {
        let s = s.strip_prefix('#').unwrap_or(s);
        if s.len() != 6 {
            return None;
        }
        let r = u8::from_str_radix(&s[0..2], 16).ok()?;
        let g = u8::from_str_radix(&s[2..4], 16).ok()?;
        let b = u8::from_str_radix(&s[4..6], 16).ok()?;
        Some(Color { r, g, b })
    }

    /// Render as `#RRGGBB`.
    pub fn to_hex(&self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

/// Font attributes. `None` fields inherit the application default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Font {
    pub name: Option<String>,
    pub size: Option<f64>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
    pub strikethrough: bool,
    pub color: Option<Color>,
}

/// Horizontal text alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum HAlign {
    #[default]
    General,
    Left,
    Center,
    Right,
}

/// Vertical text alignment.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum VAlign {
    Top,
    Middle,
    #[default]
    Bottom,
}

/// Cell alignment.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Alignment {
    pub horizontal: HAlign,
    pub vertical: VAlign,
    pub wrap_text: bool,
}

/// Line style for a single border edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum BorderStyle {
    #[default]
    None,
    Thin,
    Medium,
    Thick,
    Dashed,
    Dotted,
    Double,
}

/// One border edge.
#[derive(Debug, Clone, Copy, Default, PartialEq, Serialize, Deserialize)]
pub struct Border {
    pub style: BorderStyle,
    pub color: Option<Color>,
}

/// The four border edges of a cell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Borders {
    pub top: Border,
    pub bottom: Border,
    pub left: Border,
    pub right: Border,
}

/// The full visual style of a cell. `Default` is the unstyled cell.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CellStyle {
    pub font: Font,
    /// Background fill color.
    pub fill: Option<Color>,
    pub borders: Borders,
    pub alignment: Alignment,
    /// Number-format code (e.g. `"#,##0.00"`). `None`/`General` renders the raw
    /// value.
    pub number_format: Option<String>,
}

impl CellStyle {
    /// Whether this style is the default (no formatting). Used to avoid storing
    /// empty styles.
    pub fn is_default(&self) -> bool {
        *self == CellStyle::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_hex_roundtrip() {
        let c = Color::from_hex("#1A2B3C").unwrap();
        assert_eq!(c, Color::rgb(0x1A, 0x2B, 0x3C));
        assert_eq!(c.to_hex(), "#1A2B3C");
        assert!(Color::from_hex("xyz").is_none());
    }

    #[test]
    fn default_style_is_empty() {
        assert!(CellStyle::default().is_default());
        let mut s = CellStyle::default();
        s.font.bold = true;
        assert!(!s.is_default());
    }

    #[test]
    fn alignment_defaults() {
        let a = Alignment::default();
        assert_eq!(a.horizontal, HAlign::General);
        assert_eq!(a.vertical, VAlign::Bottom);
        assert!(!a.wrap_text);
    }
}
