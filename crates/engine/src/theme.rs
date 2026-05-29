//! Workbook themes: a named color palette + default typography that front-ends
//! (desktop, web, mobile) render against. Ships a few built-ins, including the
//! glassmorphism look the project is named for.

use crate::style::Color;
use serde::{Deserialize, Serialize};

/// The resolved colors a theme provides to a renderer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Palette {
    pub background: Color,
    pub foreground: Color,
    pub grid_line: Color,
    pub header: Color,
    pub selection: Color,
    /// Accent colors for charts/conditional formatting (cycled in order).
    pub accents: Vec<Color>,
}

/// A named theme.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub default_font: String,
    pub default_font_size: f64,
    pub palette: Palette,
}

impl Default for Theme {
    fn default() -> Self {
        Theme::light()
    }
}

impl Theme {
    /// The default light theme.
    pub fn light() -> Self {
        Theme {
            name: "Light".into(),
            dark: false,
            default_font: "Inter".into(),
            default_font_size: 11.0,
            palette: Palette {
                background: Color::WHITE,
                foreground: Color::rgb(0x20, 0x20, 0x20),
                grid_line: Color::rgb(0xE0, 0xE0, 0xE0),
                header: Color::rgb(0xF3, 0xF3, 0xF3),
                selection: Color::rgb(0xCC, 0xE0, 0xFF),
                accents: accent_set(),
            },
        }
    }

    /// A dark theme.
    pub fn dark() -> Self {
        Theme {
            name: "Dark".into(),
            dark: true,
            default_font: "Inter".into(),
            default_font_size: 11.0,
            palette: Palette {
                background: Color::rgb(0x1E, 0x1E, 0x1E),
                foreground: Color::rgb(0xE8, 0xE8, 0xE8),
                grid_line: Color::rgb(0x3A, 0x3A, 0x3A),
                header: Color::rgb(0x2A, 0x2A, 0x2A),
                selection: Color::rgb(0x35, 0x4A, 0x6B),
                accents: accent_set(),
            },
        }
    }

    /// The signature glassmorphism theme: dark, translucent-friendly, neon
    /// accents.
    pub fn glass() -> Self {
        Theme {
            name: "Glass".into(),
            dark: true,
            default_font: "Inter".into(),
            default_font_size: 11.0,
            palette: Palette {
                background: Color::rgb(0x10, 0x14, 0x22),
                foreground: Color::rgb(0xF0, 0xF4, 0xFF),
                grid_line: Color::rgb(0x2A, 0x33, 0x55),
                header: Color::rgb(0x18, 0x1F, 0x38),
                selection: Color::rgb(0x3D, 0x6F, 0xFF),
                accents: vec![
                    Color::rgb(0x4C, 0xC9, 0xF0),
                    Color::rgb(0xF7, 0x25, 0x85),
                    Color::rgb(0x7B, 0x2F, 0xF7),
                    Color::rgb(0x3A, 0x86, 0xFF),
                    Color::rgb(0x06, 0xFF, 0xA5),
                    Color::rgb(0xFF, 0xBE, 0x0B),
                ],
            },
        }
    }

    /// All built-in themes.
    pub fn builtins() -> Vec<Theme> {
        vec![Theme::light(), Theme::dark(), Theme::glass()]
    }

    /// Look up a built-in theme by name (case-insensitive).
    pub fn builtin(name: &str) -> Option<Theme> {
        Theme::builtins()
            .into_iter()
            .find(|t| t.name.eq_ignore_ascii_case(name))
    }

    /// The accent color at `index`, cycling through the palette.
    pub fn accent(&self, index: usize) -> Color {
        let accents = &self.palette.accents;
        accents[index % accents.len().max(1)]
    }
}

fn accent_set() -> Vec<Color> {
    vec![
        Color::rgb(0x42, 0x6B, 0xF5),
        Color::rgb(0xF5, 0x6B, 0x42),
        Color::rgb(0x2E, 0xB8, 0x72),
        Color::rgb(0xF5, 0xC2, 0x42),
        Color::rgb(0x9B, 0x51, 0xE0),
        Color::rgb(0x1F, 0xB6, 0xC1),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtins_present_and_distinct() {
        let names: Vec<String> = Theme::builtins().into_iter().map(|t| t.name).collect();
        assert_eq!(names, ["Light", "Dark", "Glass"]);
        assert!(Theme::builtin("glass").unwrap().dark);
        assert!(!Theme::builtin("LIGHT").unwrap().dark);
        assert!(Theme::builtin("nope").is_none());
    }

    #[test]
    fn accent_cycles() {
        let t = Theme::light();
        assert_eq!(t.accent(0), t.accent(6)); // wraps after 6 accents
    }
}
