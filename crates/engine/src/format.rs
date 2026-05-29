//! Number-format engine: render a [`Value`] through an Excel-style format code.
//!
//! Supports the numeric grammar used by the overwhelming majority of real
//! formats: up to four `;`-separated sections (positive; negative; zero; text),
//! the `0` / `#` / `?` digit placeholders, `,` thousands grouping, `.` decimal
//! point, `%` scaling, quoted/`\`-escaped literals, and `_` width spacers.
//! `General` (or an empty code) falls back to the engine's default rendering.
//!
//! Date/time format codes are handled by the date layer (built on the serial
//! system) and are out of scope here.

use crate::value::Value;

/// Format a value with an Excel number-format code.
pub fn format_value(value: &Value, code: &str) -> String {
    if let Value::Error(e) = value {
        return e.code().to_string();
    }
    let code = code.trim();
    if code.is_empty() || code.eq_ignore_ascii_case("general") {
        return value.as_text();
    }

    let sections = split_sections(code);
    match value {
        Value::Number(n) => format_number_value(*n, &sections),
        Value::Bool(_) => value.as_text(),
        Value::Empty => String::new(),
        Value::Text(t) => match sections.get(3) {
            Some(sec) => apply_text_section(sec, t),
            None => t.clone(),
        },
        Value::Error(_) => unreachable!(),
    }
}

/// Split a format code into sections on `;`, ignoring separators inside quotes.
fn split_sections(code: &str) -> Vec<String> {
    let mut sections = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut chars = code.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                in_quote = !in_quote;
                current.push(c);
            }
            '\\' => {
                current.push(c);
                if let Some(next) = chars.next() {
                    current.push(next);
                }
            }
            ';' if !in_quote => {
                sections.push(std::mem::take(&mut current));
            }
            _ => current.push(c),
        }
    }
    sections.push(current);
    sections
}

fn format_number_value(n: f64, sections: &[String]) -> String {
    let negative = n < 0.0;
    let is_zero = n == 0.0;

    let idx = if is_zero && sections.len() >= 3 {
        2
    } else if negative && sections.len() >= 2 {
        1
    } else {
        0
    };
    let section = &sections[idx];
    let mut out = format_number_section(n.abs(), section);
    // Only synthesize a sign when reusing the positive section for a negative.
    if negative && idx == 0 && !is_zero {
        out.insert(0, '-');
    }
    out
}

/// A parsed numeric section.
#[derive(Default)]
struct NumericSpec {
    prefix: String,
    suffix: String,
    int_min: usize,
    decimals: usize,
    thousands: bool,
    percent: u32,
    has_placeholders: bool,
}

fn parse_section(section: &str) -> NumericSpec {
    let mut spec = NumericSpec::default();
    let mut seen_dot = false;
    let mut chars = section.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '0' | '#' | '?' => {
                spec.has_placeholders = true;
                if seen_dot {
                    spec.decimals += 1;
                } else if c == '0' {
                    spec.int_min += 1;
                }
            }
            '.' => seen_dot = true,
            ',' => spec.thousands = true,
            '%' => {
                spec.percent += 1;
                push_literal(&mut spec, '%');
            }
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    push_literal(&mut spec, q);
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    push_literal(&mut spec, next);
                }
            }
            '_' => {
                // Width spacer: render as a space, consume the sizing char.
                chars.next();
                push_literal(&mut spec, ' ');
            }
            other => push_literal(&mut spec, other),
        }
    }
    spec
}

/// Route a literal char to the prefix (before any digit placeholder) or suffix.
fn push_literal(spec: &mut NumericSpec, c: char) {
    if spec.has_placeholders {
        spec.suffix.push(c);
    } else {
        spec.prefix.push(c);
    }
}

fn format_number_section(value: f64, section: &str) -> String {
    let spec = parse_section(section);
    if !spec.has_placeholders {
        // Pure-literal section (e.g. `"paid"`).
        return format!("{}{}", spec.prefix, spec.suffix);
    }

    let scaled = value * 10f64.powi(spec.percent as i32 * 2);
    let factor = 10f64.powi(spec.decimals as i32);
    let rounded = (scaled * factor).round() / factor;

    let int_part = rounded.trunc().abs() as u64;
    let frac = rounded.abs() - int_part as f64;

    let mut int_str = int_part.to_string();
    while int_str.len() < spec.int_min {
        int_str.insert(0, '0');
    }
    if spec.thousands {
        int_str = group_thousands(&int_str);
    }

    let mut core = int_str;
    if spec.decimals > 0 {
        let frac_scaled = (frac * factor).round() as u64;
        core.push('.');
        core.push_str(&format!("{:0width$}", frac_scaled, width = spec.decimals));
    }

    format!("{}{}{}", spec.prefix, core, spec.suffix)
}

fn group_thousands(digits: &str) -> String {
    let mut grouped = String::new();
    let len = digits.len();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (len - i).is_multiple_of(3) {
            grouped.push(',');
        }
        grouped.push(ch);
    }
    grouped
}

/// Apply a text section, substituting `@` with the text.
fn apply_text_section(section: &str, text: &str) -> String {
    let mut out = String::new();
    let mut chars = section.chars();
    while let Some(c) = chars.next() {
        match c {
            '@' => out.push_str(text),
            '"' => {
                for q in chars.by_ref() {
                    if q == '"' {
                        break;
                    }
                    out.push(q);
                }
            }
            '\\' => {
                if let Some(next) = chars.next() {
                    out.push(next);
                }
            }
            other => out.push(other),
        }
    }
    if out.is_empty() {
        text.to_string()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::CellError;

    fn fmt(n: f64, code: &str) -> String {
        format_value(&Value::Number(n), code)
    }

    #[test]
    fn fixed_decimals() {
        assert_eq!(fmt(3.5, "0.00"), "3.50");
        assert_eq!(fmt(3.456, "0.00"), "3.46");
        assert_eq!(fmt(3.0, "0"), "3");
    }

    #[test]
    fn thousands_grouping_and_rounding() {
        assert_eq!(fmt(1234.5, "#,##0"), "1,235");
        assert_eq!(fmt(1234567.0, "#,##0"), "1,234,567");
        assert_eq!(fmt(1234.5, "#,##0.00"), "1,234.50");
    }

    #[test]
    fn percent_scaling() {
        assert_eq!(fmt(0.1234, "0%"), "12%");
        assert_eq!(fmt(0.1234, "0.0%"), "12.3%");
    }

    #[test]
    fn currency_prefix_and_negatives() {
        assert_eq!(fmt(1234.5, "$#,##0.00"), "$1,234.50");
        // No negative section: synthesize a leading minus.
        assert_eq!(fmt(-12.0, "0.00"), "-12.00");
        // Dedicated negative section with parentheses, no synthesized sign.
        assert_eq!(fmt(-3.0, "0.00;(0.00)"), "(3.00)");
    }

    #[test]
    fn zero_section() {
        assert_eq!(fmt(0.0, "0.00;(0.00);\"zero\""), "zero");
    }

    #[test]
    fn general_and_errors() {
        assert_eq!(format_value(&Value::Number(1.5), "General"), "1.5");
        assert_eq!(format_value(&Value::Number(1.5), ""), "1.5");
        assert_eq!(
            format_value(&Value::Error(CellError::Div0), "0.00"),
            "#DIV/0!"
        );
    }

    #[test]
    fn text_section() {
        assert_eq!(
            format_value(
                &Value::Text("hi".into()),
                "General;General;General;\"<\"@\">\""
            ),
            "<hi>"
        );
    }
}
