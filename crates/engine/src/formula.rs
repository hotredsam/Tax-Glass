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
    /// A bareword that isn't a recognized reference — a named range or an
    /// undefined name. Resolved (or rejected as `#NAME?`) at evaluation time.
    Name(String),
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

// ----------------------------------------------------------------------------
// Tokenizer
// ----------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(f64),
    Str(String),
    Word(String),
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
            Some(Token::Number(n)) => Ok(Expr::Number(n)),
            Some(Token::Str(s)) => Ok(Expr::Text(s)),
            Some(Token::LParen) => {
                let inner = self.parse_expr()?;
                self.expect(&Token::RParen)?;
                Ok(inner)
            }
            Some(Token::Word(w)) => self.parse_word(w),
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
}
