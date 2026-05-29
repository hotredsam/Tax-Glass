//! Formula language: tokenizer, abstract syntax tree, and a recursive-descent
//! parser. Evaluation lives in [`crate::eval`]; this module only turns the text
//! of a formula (without the leading `=`) into an [`Expr`].

use crate::address::{CellRange, CellRef};
use crate::error::{EngineError, Result};

/// Binary operators, lowest binding listed first in the parser.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
    Concat,
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// A parsed formula expression tree.
#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Number(f64),
    Text(String),
    Bool(bool),
    Ref(CellRef),
    Range(CellRange),
    /// A reference qualified by a sheet name (`Sheet2!A1`).
    SheetRef(String, CellRef),
    /// A range qualified by a sheet name (`Sheet2!A1:B3`).
    SheetRange(String, CellRange),
    /// A whole-column span like `A:C` — every row of columns `start..=end`.
    ColSpan {
        start: u32,
        end: u32,
    },
    /// A whole-row span like `1:5` — every column of rows `start..=end`.
    RowSpan {
        start: u32,
        end: u32,
    },
    /// A bareword that isn't a recognized reference — a named range or an
    /// undefined name. Resolved (or rejected as `#NAME?`) at evaluation time.
    Name(String),
    /// A reference that was invalidated by a structural edit (a deleted row or
    /// column). Evaluates to `#REF!`, mirroring Excel.
    RefError,
    Neg(Box<Expr>),
    /// Postfix `%` — divides by 100.
    Percent(Box<Expr>),
    Binary(BinOp, Box<Expr>, Box<Expr>),
    Func(String, Vec<Expr>),
}

/// Parse a formula body (the text after `=`) into an [`Expr`].
pub fn parse(input: &str) -> Result<Expr> {
    let tokens = tokenize(input)?;
    let mut parser = Parser { tokens, pos: 0 };
    let expr = parser.parse_expr()?;
    if parser.peek().is_some() {
        return Err(EngineError::Syntax(format!(
            "unexpected trailing input near token {}",
            parser.pos
        )));
    }
    Ok(expr)
}

/// Render an expression back to formula text (without the leading `=`).
///
/// Output re-parses to an equivalent AST. Parenthesization is conservative
/// (a sub-expression is wrapped whenever its binding is not strictly tighter
/// than its parent's), so the result is always correct, if occasionally more
/// parenthesized than a human would write.
pub fn unparse(expr: &Expr) -> String {
    render(expr, 0, RefStyle::A1)
}

/// Render an expression with references in R1C1 notation relative to `base`
/// (the cell the formula lives in). Used for the R1C1 display mode.
pub fn to_r1c1(expr: &Expr, base: CellRef) -> String {
    render(expr, 0, RefStyle::R1C1(base))
}

/// How references are rendered while unparsing.
#[derive(Clone, Copy)]
enum RefStyle {
    A1,
    R1C1(CellRef),
}

/// Binding strength used only for unparsing. Higher binds tighter.
fn precedence(expr: &Expr) -> u8 {
    match expr {
        Expr::Binary(op, _, _) => match op {
            BinOp::Eq | BinOp::Ne | BinOp::Lt | BinOp::Gt | BinOp::Le | BinOp::Ge => 1,
            BinOp::Concat => 2,
            BinOp::Add | BinOp::Sub => 3,
            BinOp::Mul | BinOp::Div => 4,
            BinOp::Pow => 5,
        },
        Expr::Neg(_) => 6,
        Expr::Percent(_) => 7,
        _ => u8::MAX, // atoms
    }
}

fn render(expr: &Expr, parent: u8, style: RefStyle) -> String {
    let prec = precedence(expr);
    let body = match expr {
        Expr::Number(n) => crate::value::format_number(*n),
        Expr::Text(t) => format!("\"{}\"", t.replace('"', "\"\"")),
        Expr::Bool(b) => if *b { "TRUE" } else { "FALSE" }.to_string(),
        Expr::Ref(r) => render_ref(*r, style),
        Expr::Range(range) => render_range(*range, style),
        Expr::SheetRef(sheet, r) => format!("{}!{}", quote_sheet(sheet), render_ref(*r, style)),
        Expr::SheetRange(sheet, range) => {
            format!("{}!{}", quote_sheet(sheet), render_range(*range, style))
        }
        Expr::ColSpan { start, end } => match style {
            RefStyle::A1 => format!(
                "{}:{}",
                crate::address::index_to_column(*start),
                crate::address::index_to_column(*end)
            ),
            RefStyle::R1C1(_) => format!("C{}:C{}", start + 1, end + 1),
        },
        Expr::RowSpan { start, end } => match style {
            RefStyle::A1 => format!("{}:{}", start + 1, end + 1),
            RefStyle::R1C1(_) => format!("R{}:R{}", start + 1, end + 1),
        },
        Expr::Name(name) => name.clone(),
        Expr::RefError => "#REF!".to_string(),
        Expr::Neg(inner) => format!("-{}", render(inner, prec, style)),
        Expr::Percent(inner) => format!("{}%", render(inner, prec, style)),
        Expr::Binary(op, lhs, rhs) => {
            format!(
                "{}{}{}",
                render(lhs, prec, style),
                binop_str(*op),
                render(rhs, prec, style)
            )
        }
        Expr::Func(name, args) => {
            let rendered: Vec<String> = args.iter().map(|a| render(a, 0, style)).collect();
            format!("{}({})", name, rendered.join(","))
        }
    };
    // Wrap when this node binds no tighter than its parent.
    if prec <= parent {
        format!("({body})")
    } else {
        body
    }
}

fn render_ref(r: CellRef, style: RefStyle) -> String {
    match style {
        RefStyle::A1 => r.to_a1(),
        RefStyle::R1C1(base) => r.to_r1c1(base),
    }
}

fn render_range(range: CellRange, style: RefStyle) -> String {
    match style {
        RefStyle::A1 => range.to_string(),
        RefStyle::R1C1(base) => {
            format!("{}:{}", range.start.to_r1c1(base), range.end.to_r1c1(base))
        }
    }
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Eq => "=",
        BinOp::Ne => "<>",
        BinOp::Lt => "<",
        BinOp::Gt => ">",
        BinOp::Le => "<=",
        BinOp::Ge => ">=",
        BinOp::Concat => "&",
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Pow => "^",
    }
}

/// Quote a sheet name for output if it isn't a bare identifier.
fn quote_sheet(name: &str) -> String {
    let simple = !name.is_empty()
        && name
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
    if simple {
        name.to_string()
    } else {
        format!("'{}'", name.replace('\'', "''"))
    }
}

/// Translate an expression's relative references by `(dcol, drow)`, as when a
/// formula is copied/filled from one cell to another. Absolute (`$`) parts stay
/// fixed. A reference shifted to a negative coordinate becomes `#REF!`.
pub fn translate(expr: &Expr, dcol: i64, drow: i64) -> Expr {
    match expr {
        Expr::Ref(r) => match shift_ref(*r, dcol, drow) {
            Some(nr) => Expr::Ref(nr),
            None => Expr::RefError,
        },
        Expr::Range(range) => match shift_range(*range, dcol, drow) {
            Some(nr) => Expr::Range(nr),
            None => Expr::RefError,
        },
        Expr::SheetRef(sheet, r) => match shift_ref(*r, dcol, drow) {
            Some(nr) => Expr::SheetRef(sheet.clone(), nr),
            None => Expr::RefError,
        },
        Expr::SheetRange(sheet, range) => match shift_range(*range, dcol, drow) {
            Some(nr) => Expr::SheetRange(sheet.clone(), nr),
            None => Expr::RefError,
        },
        Expr::ColSpan { start, end } => {
            match (shift_index(*start, dcol), shift_index(*end, dcol)) {
                (Some(s), Some(e)) => Expr::ColSpan { start: s, end: e },
                _ => Expr::RefError,
            }
        }
        Expr::RowSpan { start, end } => {
            match (shift_index(*start, drow), shift_index(*end, drow)) {
                (Some(s), Some(e)) => Expr::RowSpan { start: s, end: e },
                _ => Expr::RefError,
            }
        }
        Expr::Neg(inner) => Expr::Neg(Box::new(translate(inner, dcol, drow))),
        Expr::Percent(inner) => Expr::Percent(Box::new(translate(inner, dcol, drow))),
        Expr::Binary(op, a, b) => Expr::Binary(
            *op,
            Box::new(translate(a, dcol, drow)),
            Box::new(translate(b, dcol, drow)),
        ),
        Expr::Func(name, args) => Expr::Func(
            name.clone(),
            args.iter().map(|a| translate(a, dcol, drow)).collect(),
        ),
        other => other.clone(),
    }
}

fn shift_index(idx: u32, delta: i64) -> Option<u32> {
    let v = idx as i64 + delta;
    if v < 0 {
        None
    } else {
        Some(v as u32)
    }
}

fn shift_ref(r: CellRef, dcol: i64, drow: i64) -> Option<CellRef> {
    let col = if r.col_abs {
        r.col as i64
    } else {
        r.col as i64 + dcol
    };
    let row = if r.row_abs {
        r.row as i64
    } else {
        r.row as i64 + drow
    };
    if col < 0 || row < 0 {
        return None;
    }
    Some(CellRef {
        col: col as u32,
        row: row as u32,
        ..r
    })
}

fn shift_range(range: CellRange, dcol: i64, drow: i64) -> Option<CellRange> {
    Some(CellRange {
        start: shift_ref(range.start, dcol, drow)?,
        end: shift_ref(range.end, dcol, drow)?,
    })
}

// ----------------------------------------------------------------------------
// Tokenizer
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Str(String),
    Word(String),
    /// A single-quoted sheet name, e.g. `'My Sheet'`.
    Quoted(String),
    /// `!` — the sheet/reference separator.
    Bang,
    Plus,
    Minus,
    Star,
    Slash,
    Caret,
    Amp,
    Percent,
    LParen,
    RParen,
    Comma,
    Colon,
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

fn tokenize(input: &str) -> Result<Vec<Token>> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            ' ' | '\t' | '\r' | '\n' => i += 1,
            '+' => {
                tokens.push(Token::Plus);
                i += 1;
            }
            '-' => {
                tokens.push(Token::Minus);
                i += 1;
            }
            '*' => {
                tokens.push(Token::Star);
                i += 1;
            }
            '/' => {
                tokens.push(Token::Slash);
                i += 1;
            }
            '^' => {
                tokens.push(Token::Caret);
                i += 1;
            }
            '&' => {
                tokens.push(Token::Amp);
                i += 1;
            }
            '%' => {
                tokens.push(Token::Percent);
                i += 1;
            }
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            ':' => {
                tokens.push(Token::Colon);
                i += 1;
            }
            '!' => {
                tokens.push(Token::Bang);
                i += 1;
            }
            '\'' => {
                // Single-quoted sheet name with '' as an escaped quote.
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(EngineError::Syntax("unterminated sheet name".into())),
                        Some('\'') => {
                            if chars.get(i + 1) == Some(&'\'') {
                                s.push('\'');
                                i += 2;
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Quoted(s));
            }
            '=' => {
                tokens.push(Token::Eq);
                i += 1;
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Le);
                    i += 2;
                } else if chars.get(i + 1) == Some(&'>') {
                    tokens.push(Token::Ne);
                    i += 2;
                } else {
                    tokens.push(Token::Lt);
                    i += 1;
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Ge);
                    i += 2;
                } else {
                    tokens.push(Token::Gt);
                    i += 1;
                }
            }
            '"' => {
                // String literal with "" as an escaped quote.
                let mut s = String::new();
                i += 1;
                loop {
                    match chars.get(i) {
                        None => return Err(EngineError::Syntax("unterminated string".into())),
                        Some('"') => {
                            if chars.get(i + 1) == Some(&'"') {
                                s.push('"');
                                i += 2;
                            } else {
                                i += 1;
                                break;
                            }
                        }
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Str(s));
            }
            c if c.is_ascii_digit() || (c == '.' && next_is_digit(&chars, i)) => {
                let start = i;
                while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '.') {
                    i += 1;
                }
                // Scientific notation: 1e10, 2.5E-3.
                if i < chars.len() && (chars[i] == 'e' || chars[i] == 'E') {
                    i += 1;
                    if i < chars.len() && (chars[i] == '+' || chars[i] == '-') {
                        i += 1;
                    }
                    while i < chars.len() && chars[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let text: String = chars[start..i].iter().collect();
                let n = text
                    .parse::<f64>()
                    .map_err(|_| EngineError::Syntax(format!("invalid number {text:?}")))?;
                tokens.push(Token::Number(n));
            }
            c if c.is_ascii_alphabetic() || c == '$' || c == '_' => {
                let start = i;
                while i < chars.len()
                    && (chars[i].is_ascii_alphanumeric()
                        || chars[i] == '$'
                        || chars[i] == '_'
                        || chars[i] == '.')
                {
                    i += 1;
                }
                let text: String = chars[start..i].iter().collect();
                tokens.push(Token::Word(text));
            }
            other => {
                return Err(EngineError::Syntax(format!(
                    "unexpected character {other:?}"
                )))
            }
        }
    }
    Ok(tokens)
}

fn next_is_digit(chars: &[char], i: usize) -> bool {
    chars.get(i + 1).is_some_and(|c| c.is_ascii_digit())
}

// ----------------------------------------------------------------------------
// Parser
// ----------------------------------------------------------------------------

struct Parser {
    tokens: Vec<Token>,
    pos: usize,
}

impl Parser {
    fn peek(&self) -> Option<&Token> {
        self.tokens.get(self.pos)
    }

    fn advance(&mut self) -> Option<Token> {
        let t = self.tokens.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn expect(&mut self, want: &Token) -> Result<()> {
        if self.peek() == Some(want) {
            self.pos += 1;
            Ok(())
        } else {
            Err(EngineError::Syntax(format!("expected {want:?}")))
        }
    }

    fn parse_expr(&mut self) -> Result<Expr> {
        self.parse_comparison()
    }

    fn parse_comparison(&mut self) -> Result<Expr> {
        let mut left = self.parse_concat()?;
        while let Some(op) = self.peek().and_then(comparison_op) {
            self.advance();
            let right = self.parse_concat()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_concat(&mut self) -> Result<Expr> {
        let mut left = self.parse_additive()?;
        while self.peek() == Some(&Token::Amp) {
            self.advance();
            let right = self.parse_additive()?;
            left = Expr::Binary(BinOp::Concat, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_additive(&mut self) -> Result<Expr> {
        let mut left = self.parse_term()?;
        loop {
            let op = match self.peek() {
                Some(Token::Plus) => BinOp::Add,
                Some(Token::Minus) => BinOp::Sub,
                _ => break,
            };
            self.advance();
            let right = self.parse_term()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    fn parse_term(&mut self) -> Result<Expr> {
        let mut left = self.parse_factor()?;
        loop {
            let op = match self.peek() {
                Some(Token::Star) => BinOp::Mul,
                Some(Token::Slash) => BinOp::Div,
                _ => break,
            };
            self.advance();
            let right = self.parse_factor()?;
            left = Expr::Binary(op, Box::new(left), Box::new(right));
        }
        Ok(left)
    }

    /// Exponentiation, right-associative.
    fn parse_factor(&mut self) -> Result<Expr> {
        let base = self.parse_unary()?;
        if self.peek() == Some(&Token::Caret) {
            self.advance();
            let exp = self.parse_factor()?;
            Ok(Expr::Binary(BinOp::Pow, Box::new(base), Box::new(exp)))
        } else {
            Ok(base)
        }
    }

    fn parse_unary(&mut self) -> Result<Expr> {
        match self.peek() {
            Some(Token::Minus) => {
                self.advance();
                Ok(Expr::Neg(Box::new(self.parse_unary()?)))
            }
            Some(Token::Plus) => {
                self.advance();
                self.parse_unary()
            }
            _ => self.parse_postfix(),
        }
    }

    fn parse_postfix(&mut self) -> Result<Expr> {
        let mut expr = self.parse_primary()?;
        while self.peek() == Some(&Token::Percent) {
            self.advance();
            expr = Expr::Percent(Box::new(expr));
        }
        Ok(expr)
    }

    fn parse_primary(&mut self) -> Result<Expr> {
        match self.advance() {
            Some(Token::Number(n)) => {
                // Whole-row span: N ':' M (1-based row numbers).
                if self.peek() == Some(&Token::Colon) {
                    if let Some(Token::Number(m)) = self.tokens.get(self.pos + 1).cloned() {
                        if let (Some(r1), Some(r2)) = (row_index(n), row_index(m)) {
                            self.advance(); // ':'
                            self.advance(); // M
                            return Ok(Expr::RowSpan {
                                start: r1.min(r2),
                                end: r1.max(r2),
                            });
                        }
                    }
                }
                Ok(Expr::Number(n))
            }
            Some(Token::Str(s)) => Ok(Expr::Text(s)),
            Some(Token::LParen) => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::Word(w)) => self.parse_word(w),
            Some(Token::Quoted(name)) => {
                // A quoted name is only meaningful as a sheet qualifier.
                self.expect(&Token::Bang)?;
                self.parse_sheet_qualified(name)
            }
            other => Err(EngineError::Syntax(format!("unexpected token {other:?}"))),
        }
    }

    /// A word can begin a function call, a cell reference, a range, a boolean,
    /// or a bare name.
    fn parse_word(&mut self, word: String) -> Result<Expr> {
        if self.peek() == Some(&Token::LParen) {
            self.advance();
            let args = self.parse_args()?;
            self.expect(&Token::RParen)?;
            return Ok(Expr::Func(word.to_ascii_uppercase(), args));
        }

        // Sheet-qualified reference: WORD '!' ...
        if self.peek() == Some(&Token::Bang) {
            self.advance();
            return self.parse_sheet_qualified(word);
        }

        // Whole-column span: COL ':' COL (e.g. A:A, A:C) — only when both sides
        // are column-letter-only words.
        if self.peek() == Some(&Token::Colon) {
            if let Some(c1) = column_only(&word) {
                if let Some(Token::Word(w2)) = self.tokens.get(self.pos + 1).cloned() {
                    if let Some(c2) = column_only(&w2) {
                        self.advance(); // ':'
                        self.advance(); // second column
                        return Ok(Expr::ColSpan {
                            start: c1.min(c2),
                            end: c1.max(c2),
                        });
                    }
                }
            }
        }

        // Range: WORD ':' WORD where both sides are references.
        if self.peek() == Some(&Token::Colon) {
            if let Ok(start) = CellRef::parse(&word) {
                self.advance(); // consume ':'
                match self.advance() {
                    Some(Token::Word(end_word)) => {
                        let end = CellRef::parse(&end_word)?;
                        return Ok(Expr::Range(CellRange::new(start, end)));
                    }
                    other => {
                        return Err(EngineError::Syntax(format!(
                            "expected reference after ':', got {other:?}"
                        )))
                    }
                }
            }
        }

        match word.to_ascii_uppercase().as_str() {
            "TRUE" => Ok(Expr::Bool(true)),
            "FALSE" => Ok(Expr::Bool(false)),
            _ => match CellRef::parse(&word) {
                Ok(r) => Ok(Expr::Ref(r)),
                Err(_) => Ok(Expr::Name(word)),
            },
        }
    }

    /// Parse the reference part of a sheet-qualified reference, with the `!`
    /// already consumed. Produces a `SheetRef` or `SheetRange`.
    fn parse_sheet_qualified(&mut self, sheet: String) -> Result<Expr> {
        let start = match self.advance() {
            Some(Token::Word(w)) => CellRef::parse(&w)?,
            other => {
                return Err(EngineError::Syntax(format!(
                    "expected reference after '!', got {other:?}"
                )))
            }
        };
        if self.peek() == Some(&Token::Colon) {
            self.advance();
            match self.advance() {
                Some(Token::Word(end_word)) => {
                    let end = CellRef::parse(&end_word)?;
                    Ok(Expr::SheetRange(sheet, CellRange::new(start, end)))
                }
                other => Err(EngineError::Syntax(format!(
                    "expected reference after ':', got {other:?}"
                ))),
            }
        } else {
            Ok(Expr::SheetRef(sheet, start))
        }
    }

    fn parse_args(&mut self) -> Result<Vec<Expr>> {
        let mut args = Vec::new();
        if self.peek() == Some(&Token::RParen) {
            return Ok(args);
        }
        loop {
            args.push(self.parse_expr()?);
            match self.peek() {
                Some(Token::Comma) => {
                    self.advance();
                }
                _ => break,
            }
        }
        Ok(args)
    }
}

/// If `word` is column letters only (optionally `$`-prefixed), return its
/// zero-based column index. Rejects anything containing digits (so `A1` is not
/// treated as a column).
fn column_only(word: &str) -> Option<u32> {
    let letters = word.strip_prefix('$').unwrap_or(word);
    if letters.is_empty() || !letters.chars().all(|c| c.is_ascii_alphabetic()) {
        return None;
    }
    crate::address::column_to_index(letters)
}

/// Convert a 1-based row number literal to a zero-based index, if it is a
/// positive integer.
fn row_index(n: f64) -> Option<u32> {
    if n.fract() == 0.0 && n >= 1.0 && n <= u32::MAX as f64 {
        Some(n as u32 - 1)
    } else {
        None
    }
}

fn comparison_op(t: &Token) -> Option<BinOp> {
    Some(match t {
        Token::Eq => BinOp::Eq,
        Token::Ne => BinOp::Ne,
        Token::Lt => BinOp::Lt,
        Token::Gt => BinOp::Gt,
        Token::Le => BinOp::Le,
        Token::Ge => BinOp::Ge,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Expr {
        parse(s).unwrap()
    }

    #[test]
    fn parses_arithmetic_precedence() {
        // 1 + 2 * 3 -> 1 + (2 * 3)
        let e = p("1+2*3");
        match e {
            Expr::Binary(BinOp::Add, _, rhs) => {
                assert!(matches!(*rhs, Expr::Binary(BinOp::Mul, _, _)));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn power_is_right_associative() {
        // 2^3^2 -> 2^(3^2)
        let e = p("2^3^2");
        match e {
            Expr::Binary(BinOp::Pow, _, rhs) => {
                assert!(matches!(*rhs, Expr::Binary(BinOp::Pow, _, _)));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_function_with_range() {
        let e = p("SUM(A1:B2)");
        match e {
            Expr::Func(name, args) => {
                assert_eq!(name, "SUM");
                assert_eq!(args.len(), 1);
                assert!(matches!(args[0], Expr::Range(_)));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_nested_functions_and_commas() {
        let e = p("IF(A1>0, MAX(B1,C1), 0)");
        match e {
            Expr::Func(name, args) => {
                assert_eq!(name, "IF");
                assert_eq!(args.len(), 3);
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_strings_and_concat() {
        let e = p("\"a\" & \"b\"");
        assert!(matches!(e, Expr::Binary(BinOp::Concat, _, _)));
    }

    #[test]
    fn parses_unary_and_percent() {
        assert!(matches!(p("-5"), Expr::Neg(_)));
        assert!(matches!(p("50%"), Expr::Percent(_)));
    }

    #[test]
    fn rejects_garbage() {
        assert!(parse("1 +").is_err());
        assert!(parse("(1+2").is_err());
        assert!(parse("\"oops").is_err());
        assert!(parse("1 2").is_err());
    }

    #[test]
    fn parses_sheet_qualified_ref_and_range() {
        match p("Sheet2!A1") {
            Expr::SheetRef(name, r) => {
                assert_eq!(name, "Sheet2");
                assert_eq!(r.to_a1(), "A1");
            }
            other => panic!("unexpected: {other:?}"),
        }
        match p("Data!A1:B3") {
            Expr::SheetRange(name, range) => {
                assert_eq!(name, "Data");
                assert_eq!(range.to_string(), "A1:B3");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_quoted_sheet_name() {
        match p("'My Sheet'!B2") {
            Expr::SheetRef(name, r) => {
                assert_eq!(name, "My Sheet");
                assert_eq!(r.to_a1(), "B2");
            }
            other => panic!("unexpected: {other:?}"),
        }
    }

    #[test]
    fn parses_whole_column_and_row_spans() {
        assert_eq!(p("A:A"), Expr::ColSpan { start: 0, end: 0 });
        assert_eq!(p("A:C"), Expr::ColSpan { start: 0, end: 2 });
        assert_eq!(p("2:5"), Expr::RowSpan { start: 1, end: 4 });
        match p("SUM(B:B)") {
            Expr::Func(name, args) => {
                assert_eq!(name, "SUM");
                assert_eq!(args[0], Expr::ColSpan { start: 1, end: 1 });
            }
            other => panic!("unexpected: {other:?}"),
        }
        // A normal cell range still parses as a range, not a span.
        assert!(matches!(p("A1:A3"), Expr::Range(_)));
    }

    #[test]
    fn span_unparse_roundtrips() {
        for s in ["A:A", "A:C", "2:5"] {
            assert_eq!(unparse(&p(s)), s);
        }
    }

    #[test]
    fn renders_formula_in_r1c1() {
        let base = CellRef::parse("C3").unwrap();
        // A1 from C3 -> R[-2]C[-2]; $A$1 -> R1C1.
        assert_eq!(to_r1c1(&p("A1+$A$1"), base), "R[-2]C[-2]+R1C1");
        // Range and a whole column.
        assert_eq!(to_r1c1(&p("SUM(A1:A3)"), base), "SUM(R[-2]C[-2]:RC[-2])");
        assert_eq!(to_r1c1(&p("SUM(B:B)"), base), "SUM(C2:C2)");
    }

    #[test]
    fn sheet_ref_inside_function() {
        match p("SUM(Sheet2!A1:A3, B1)") {
            Expr::Func(name, args) => {
                assert_eq!(name, "SUM");
                assert!(matches!(args[0], Expr::SheetRange(_, _)));
                assert!(matches!(args[1], Expr::Ref(_)));
            }
            other => panic!("unexpected: {other:?}"),
        }
    }
}
