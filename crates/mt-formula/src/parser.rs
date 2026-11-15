//! From text to tree.
//!
//! ```text
//! formula    := comparison
//! comparison := concat [ ("=" | "<>" | "<" | "<=" | ">" | ">=") concat ]
//! concat     := sum { "&" sum }
//! sum        := product { ("+" | "-") product }
//! product    := unary { ("*" | "/") unary }
//! unary      := ("-" | "+") unary | postfix
//! postfix    := primary { "%" }
//! primary    := number | "text" | TRUE | FALSE | CurrentValue
//!             | {Field} | {Link}.{Field} | NAME "(" [ formula { "," formula } ] ")"
//!             | "(" formula ")"
//! ```
//!
//! * Numbers are written with a decimal point (`1234.5`). Text is in double quotes; a quote
//!   inside text is doubled (`"say ""hi"""`).
//! * A field is its name in braces. Inside the braces `\}` is a brace and `\\` a backslash.
//! * Function names, `TRUE`, `FALSE` and `CurrentValue` ignore case.
//! * Comparisons do not chain: `1 < {x} < 10` is refused, because it would compare the
//!   result of `1 < {x}` with 10. Write `AND(1 < {x}, {x} < 10)`.

use crate::ast::{
    BinaryOp, Expr, Function, LogicFn, UnaryOp, MAX_DEPTH, MAX_NODES, MAX_SOURCE_CHARS,
};
use crate::error::FormulaError;
use crate::schema::Schema;
use mt_core::decimal::Decimal;

#[derive(Debug, Clone, PartialEq)]
enum Token {
    Number(Decimal),
    Text(String),
    Field(String),
    Name(String),
    LeftParen,
    RightParen,
    Comma,
    Dot,
    Plus,
    Minus,
    Percent,
    Operator(BinaryOp),
    End,
}

impl Token {
    fn describe(&self) -> String {
        match self {
            Token::Number(n) => format!("the number {n}"),
            Token::Text(_) => "a text".to_owned(),
            Token::Field(name) => format!("the field {{{name}}}"),
            Token::Name(name) => format!("`{name}`"),
            Token::LeftParen => "`(`".to_owned(),
            Token::RightParen => "`)`".to_owned(),
            Token::Comma => "`,`".to_owned(),
            Token::Dot => "`.`".to_owned(),
            Token::Plus => "`+`".to_owned(),
            Token::Minus => "`-`".to_owned(),
            Token::Percent => "`%`".to_owned(),
            Token::Operator(op) => format!("`{}`", op.symbol()),
            Token::End => "the end of the formula".to_owned(),
        }
    }
}

/// Splits the text into tokens, each with the character position where it starts.
fn tokenize(source: &str) -> Result<Vec<(Token, usize)>, FormulaError> {
    let chars: Vec<char> = source.chars().collect();
    if chars.len() > MAX_SOURCE_CHARS {
        return Err(FormulaError::Limit("more than 10,000 characters"));
    }
    let mut tokens = Vec::new();
    let mut i = 0_usize;
    let at = |i: usize| chars.get(i).copied();
    while let Some(c) = at(i) {
        let start = i;
        i = i.saturating_add(1);
        let token = match c {
            ' ' | '\t' | '\n' | '\r' => continue,
            '(' => Token::LeftParen,
            ')' => Token::RightParen,
            ',' => Token::Comma,
            '.' => Token::Dot,
            '+' => Token::Plus,
            '-' => Token::Minus,
            '%' => Token::Percent,
            '*' => Token::Operator(BinaryOp::Multiply),
            '/' => Token::Operator(BinaryOp::Divide),
            '&' => Token::Operator(BinaryOp::Concat),
            '=' => Token::Operator(BinaryOp::Equal),
            '<' => match at(i) {
                Some('=') => {
                    i = i.saturating_add(1);
                    Token::Operator(BinaryOp::LessOrEqual)
                }
                Some('>') => {
                    i = i.saturating_add(1);
                    Token::Operator(BinaryOp::NotEqual)
                }
                Some(_) | None => Token::Operator(BinaryOp::Less),
            },
            '>' => {
                if at(i) == Some('=') {
                    i = i.saturating_add(1);
                    Token::Operator(BinaryOp::GreaterOrEqual)
                } else {
                    Token::Operator(BinaryOp::Greater)
                }
            }
            '"' => {
                let mut text = String::new();
                loop {
                    match at(i) {
                        None => {
                            return Err(FormulaError::syntax(
                                start,
                                "this text is not closed with `\"`",
                            ))
                        }
                        Some('"') if at(i.saturating_add(1)) == Some('"') => {
                            text.push('"');
                            i = i.saturating_add(2);
                        }
                        Some('"') => {
                            i = i.saturating_add(1);
                            break;
                        }
                        Some(other) => {
                            text.push(other);
                            i = i.saturating_add(1);
                        }
                    }
                }
                Token::Text(text)
            }
            '{' => {
                let mut name = String::new();
                loop {
                    match at(i) {
                        None => {
                            return Err(FormulaError::syntax(
                                start,
                                "this field name is not closed with `}`",
                            ))
                        }
                        Some('\\') => match at(i.saturating_add(1)) {
                            Some(escaped @ ('}' | '\\')) => {
                                name.push(escaped);
                                i = i.saturating_add(2);
                            }
                            Some(_) | None => {
                                return Err(FormulaError::syntax(
                                    i,
                                    "inside a field name, `\\` must be followed by `}` or `\\`",
                                ));
                            }
                        },
                        Some('}') => {
                            i = i.saturating_add(1);
                            break;
                        }
                        Some(other) => {
                            name.push(other);
                            i = i.saturating_add(1);
                        }
                    }
                }
                if name.trim().is_empty() {
                    return Err(FormulaError::syntax(
                        start,
                        "a field name is missing between `{` and `}`",
                    ));
                }
                Token::Field(name)
            }
            digit if digit.is_ascii_digit() => {
                while at(i).is_some_and(|d| d.is_ascii_digit()) {
                    i = i.saturating_add(1);
                }
                // A decimal point belongs to the number only when a digit follows.
                if at(i) == Some('.') && at(i.saturating_add(1)).is_some_and(|d| d.is_ascii_digit())
                {
                    i = i.saturating_add(1);
                    while at(i).is_some_and(|d| d.is_ascii_digit()) {
                        i = i.saturating_add(1);
                    }
                }
                let text: String = chars
                    .iter()
                    .skip(start)
                    .take(i.saturating_sub(start))
                    .collect();
                let number = text
                    .parse::<Decimal>()
                    .map_err(|_| FormulaError::syntax(start, "this number has too many digits"))?;
                Token::Number(number)
            }
            letter if letter.is_ascii_alphabetic() || letter == '_' => {
                while at(i).is_some_and(|d| d.is_ascii_alphanumeric() || d == '_') {
                    i = i.saturating_add(1);
                }
                Token::Name(
                    chars
                        .iter()
                        .skip(start)
                        .take(i.saturating_sub(start))
                        .collect(),
                )
            }
            other => {
                return Err(FormulaError::syntax(
                    start,
                    format!("unexpected character `{other}`"),
                ));
            }
        };
        tokens.push((token, start));
    }
    tokens.push((Token::End, chars.len()));
    Ok(tokens)
}

struct Parser<'s> {
    tokens: Vec<(Token, usize)>,
    next: usize,
    depth: usize,
    schema: &'s dyn Schema,
}

impl Parser<'_> {
    fn peek(&self) -> &Token {
        self.tokens
            .get(self.next)
            .map_or(&Token::End, |(token, _)| token)
    }

    fn peek_second(&self) -> &Token {
        self.tokens
            .get(self.next.saturating_add(1))
            .map_or(&Token::End, |(token, _)| token)
    }

    fn position(&self) -> usize {
        self.tokens
            .get(self.next)
            .or_else(|| self.tokens.last())
            .map_or(0, |(_, position)| *position)
    }

    fn advance(&mut self) -> Token {
        let token = self.peek().clone();
        self.next = self.next.saturating_add(1);
        token
    }

    fn unexpected(&self, wanted: &str) -> FormulaError {
        FormulaError::syntax(
            self.position(),
            format!("expected {wanted}, found {}", self.peek().describe()),
        )
    }

    fn enter(&mut self) -> Result<(), FormulaError> {
        self.depth = self.depth.saturating_add(1);
        if self.depth > MAX_DEPTH {
            return Err(FormulaError::Limit("nested more than 64 levels deep"));
        }
        Ok(())
    }

    fn leave(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    /// Parses operators that bind at least as tightly as `min_precedence`.
    fn expression(&mut self, min_precedence: u8) -> Result<Expr, FormulaError> {
        self.enter()?;
        let mut left = self.unary()?;
        let mut compared = false;
        loop {
            let Token::Operator(op) = self.peek().clone() else {
                // `+` and `-` are their own tokens because they can also be signs.
                let op = match self.peek() {
                    Token::Plus => BinaryOp::Add,
                    Token::Minus => BinaryOp::Subtract,
                    Token::Number(_)
                    | Token::Text(_)
                    | Token::Field(_)
                    | Token::Name(_)
                    | Token::LeftParen
                    | Token::RightParen
                    | Token::Comma
                    | Token::Dot
                    | Token::Percent
                    | Token::Operator(_)
                    | Token::End => break,
                };
                if op.precedence() < min_precedence {
                    break;
                }
                self.advance();
                let right = self.expression(op.precedence().saturating_add(1))?;
                left = Expr::Binary {
                    op,
                    left: Box::new(left),
                    right: Box::new(right),
                };
                continue;
            };
            if op.precedence() < min_precedence {
                break;
            }
            if op.is_comparison() && compared {
                return Err(FormulaError::syntax(
                    self.position(),
                    "comparisons cannot be chained; compare two values at a time and combine with AND(…)",
                ));
            }
            compared = op.is_comparison();
            self.advance();
            let right = self.expression(op.precedence().saturating_add(1))?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
        self.leave();
        Ok(left)
    }

    fn unary(&mut self) -> Result<Expr, FormulaError> {
        match self.peek() {
            Token::Plus => {
                self.advance();
                self.unary()
            }
            Token::Minus => {
                self.advance();
                // `-5` is the number minus five; `-5%` is the negation of five percent.
                if let (Token::Number(n), false) = (
                    self.peek().clone(),
                    matches!(self.peek_second(), Token::Percent),
                ) {
                    self.advance();
                    return Ok(Expr::Number(n.checked_neg_or_self()));
                }
                self.enter()?;
                let operand = self.unary()?;
                self.leave();
                Ok(Expr::Unary {
                    op: UnaryOp::Negate,
                    operand: Box::new(operand),
                })
            }
            Token::Number(_)
            | Token::Text(_)
            | Token::Field(_)
            | Token::Name(_)
            | Token::LeftParen
            | Token::RightParen
            | Token::Comma
            | Token::Dot
            | Token::Percent
            | Token::Operator(_)
            | Token::End => self.postfix(),
        }
    }

    fn postfix(&mut self) -> Result<Expr, FormulaError> {
        let mut expr = self.primary()?;
        while matches!(self.peek(), Token::Percent) {
            self.advance();
            expr = Expr::Unary {
                op: UnaryOp::Percent,
                operand: Box::new(expr),
            };
        }
        Ok(expr)
    }

    fn primary(&mut self) -> Result<Expr, FormulaError> {
        let position = self.position();
        match self.advance() {
            Token::Number(n) => Ok(Expr::Number(n)),
            Token::Text(text) => Ok(Expr::Text(text)),
            Token::Field(name) => self.field(&name, position),
            Token::Name(name) => self.name(&name, position),
            Token::LeftParen => {
                let inner = self.expression(1)?;
                if !matches!(self.peek(), Token::RightParen) {
                    return Err(self.unexpected("`)`"));
                }
                self.advance();
                Ok(inner)
            }
            Token::RightParen
            | Token::Comma
            | Token::Dot
            | Token::Plus
            | Token::Minus
            | Token::Percent
            | Token::Operator(_)
            | Token::End => {
                // Step back so that the message names the offending token.
                self.next = self.next.saturating_sub(1);
                Err(self.unexpected("a value"))
            }
        }
    }

    fn field(&mut self, name: &str, position: usize) -> Result<Expr, FormulaError> {
        let unknown = |name: String| FormulaError::UnknownField { name, position };
        let field = self
            .schema
            .field_by_name(name)
            .ok_or_else(|| unknown(name.to_owned()))?;
        if !matches!(self.peek(), Token::Dot) {
            return Ok(Expr::Field(field));
        }
        self.advance();
        let Token::Field(linked_name) = self.peek().clone() else {
            return Err(self.unexpected("a field of the linked table, such as `{Amount}`"));
        };
        self.advance();
        let linked = self
            .schema
            .linked_field_by_name(field, &linked_name)
            .ok_or_else(|| unknown(format!("{name}.{linked_name}")))?;
        Ok(Expr::Through {
            link: field,
            field: linked,
        })
    }

    fn name(&mut self, name: &str, position: usize) -> Result<Expr, FormulaError> {
        if !matches!(self.peek(), Token::LeftParen) {
            return match name.to_ascii_uppercase().as_str() {
                "TRUE" => Ok(Expr::Bool(true)),
                "FALSE" => Ok(Expr::Bool(false)),
                "CURRENTVALUE" => Ok(Expr::CurrentValue),
                _ => Err(FormulaError::syntax(
                    position,
                    format!("`{name}` is not a value; put field names in braces, like {{{name}}}, and text in quotes"),
                )),
            };
        }
        let function = Function::by_name(name).ok_or_else(|| FormulaError::UnknownFunction {
            name: name.to_owned(),
            position,
        })?;
        self.advance();
        let mut args = Vec::new();
        if matches!(self.peek(), Token::RightParen) {
            self.advance();
        } else {
            loop {
                args.push(self.expression(1)?);
                match self.peek() {
                    Token::Comma => {
                        self.advance();
                    }
                    Token::RightParen => {
                        self.advance();
                        break;
                    }
                    Token::Number(_)
                    | Token::Text(_)
                    | Token::Field(_)
                    | Token::Name(_)
                    | Token::LeftParen
                    | Token::Dot
                    | Token::Plus
                    | Token::Minus
                    | Token::Percent
                    | Token::Operator(_)
                    | Token::End => return Err(self.unexpected("`,` or `)`")),
                }
            }
        }
        check_arity(function, args.len())?;
        Ok(Expr::Call { function, args })
    }
}

/// Verifies the number of arguments of a call.
pub(crate) fn check_arity(function: Function, found: usize) -> Result<(), FormulaError> {
    let too_few = found < function.min_args();
    let too_many = function.max_args().is_some_and(|max| found > max);
    // IFS takes pairs of condition and value.
    let is_ifs = function == Function::Logic(LogicFn::Ifs);
    let unpaired = is_ifs && found.checked_rem(2) != Some(0);
    if too_few || too_many || unpaired {
        let expected = if is_ifs {
            "an even number of".to_owned()
        } else {
            function.arity_text()
        };
        return Err(FormulaError::Arity {
            function: function.name(),
            expected,
            found,
        });
    }
    Ok(())
}

trait NegateLiteral {
    fn checked_neg_or_self(self) -> Self;
}

impl NegateLiteral for Decimal {
    /// The negated number (a decimal can always be negated; zero stays zero).
    fn checked_neg_or_self(mut self) -> Self {
        self.set_sign_negative(!self.is_zero());
        self
    }
}

/// Parses a formula, resolving field names with `schema`.
///
/// # Errors
/// [`FormulaError::Syntax`], [`FormulaError::UnknownField`],
/// [`FormulaError::UnknownFunction`], [`FormulaError::Arity`] or [`FormulaError::Limit`].
pub fn parse(source: &str, schema: &dyn Schema) -> Result<Expr, FormulaError> {
    let tokens = tokenize(source)?;
    if tokens.len() <= 1 {
        return Err(FormulaError::syntax(0, "the formula is empty"));
    }
    let mut parser = Parser {
        tokens,
        next: 0,
        depth: 0,
        schema,
    };
    let expr = parser.expression(1)?;
    if !matches!(parser.peek(), Token::End) {
        return Err(parser.unexpected("an operator or the end of the formula"));
    }
    if expr.node_count() > MAX_NODES {
        return Err(FormulaError::Limit("more than 2,000 parts"));
    }
    Ok(expr)
}
