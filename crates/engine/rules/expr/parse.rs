//! Reading the text of a number into its syntax tree, refusing what can never be a value.

use crate::rules::expr::{BinOp, Expr, Func, Keep};
use crate::rules::grammar;

impl Expr {
    /// Parse one expression. A literal zero divisor, a dice group with no dice or faces, more than
    /// `MAX_DICE` dice and a literal past `i64::MAX` fail here.
    pub(crate) fn parse(src: &str) -> Result<Expr, String> {
        let mut parser = Parser { src, pos: 0 };
        parser.skip_whitespace();
        if parser.is_at_end() {
            return Err(parser.not_a_number());
        }
        let expr = parser.expr()?;
        parser.skip_whitespace();
        if !parser.is_at_end() {
            return Err(parser.not_a_number());
        }
        if divides_by_zero(&expr) {
            return Err(format!("expression '{src}' divides by zero"));
        }
        Ok(expr)
    }
}

fn divides_by_zero(expr: &Expr) -> bool {
    match expr {
        Expr::Binary {
            op: BinOp::Div,
            right,
            ..
        } if matches!(right.as_ref(), Expr::Num(0)) => true,
        Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
            divides_by_zero(left) || divides_by_zero(right)
        }
        Expr::Neg(inner) => divides_by_zero(inner),
        _ => false,
    }
}

/// Most dice one group may roll. A larger count is almost always a typo.
const MAX_DICE: u32 = 1000;

struct Parser<'src> {
    src: &'src str,
    pos: usize,
}

impl Parser<'_> {
    fn expr(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        loop {
            self.skip_whitespace();
            let op = if self.eat('+') {
                BinOp::Add
            } else if self.eat('-') {
                BinOp::Sub
            } else {
                return Ok(left);
            };
            let right = self.term()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            self.skip_whitespace();
            let op = if self.eat('*') {
                BinOp::Mul
            } else if self.eat('/') {
                BinOp::Div
            } else {
                return Ok(left);
            };
            let right = self.unary()?;
            left = Expr::Binary {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();
        if self.eat('-') {
            let inner = self.unary()?;
            return Ok(Expr::Neg(Box::new(inner)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        self.skip_whitespace();
        if self.eat('(') {
            let inner = self.expr()?;
            self.eat_closing_parenthesis()?;
            return Ok(inner);
        }
        if self.peek().is_ascii_digit() {
            return self.number_or_dice();
        }
        if self.peek().is_ascii_alphabetic() || self.peek() == '_' {
            return self.call_or_read();
        }
        Err(self.not_a_number())
    }

    fn number_or_dice(&mut self) -> Result<Expr, String> {
        let n = self.digits()?;
        if self.eat('d') {
            let sides = self.digits()?;
            let keep = self.keep()?;
            if n == 0 || sides == 0 || sides > u64::from(u32::MAX) {
                return Err(format!("expression '{}' rolls no dice", self.src));
            }
            if n > u64::from(MAX_DICE) {
                return Err(format!(
                    "expression '{}' rolls more than {MAX_DICE} dice",
                    self.src
                ));
            }
            return Ok(Expr::Dice {
                count: n as u32,
                sides: sides as u32,
                keep,
            });
        }
        if n > i64::MAX as u64 {
            return Err(self.not_a_number());
        }
        Ok(Expr::Num(n as i64))
    }

    fn keep(&mut self) -> Result<Option<Keep>, String> {
        if self.starts_with("kh1") {
            self.pos += 3;
            Ok(Some(Keep::High))
        } else if self.starts_with("kl1") {
            self.pos += 3;
            Ok(Some(Keep::Low))
        } else if self.peek() == 'k' {
            Err(self.not_a_number())
        } else {
            Ok(None)
        }
    }

    fn call_or_read(&mut self) -> Result<Expr, String> {
        let start = self.pos;
        let word = self.ident()?;
        self.skip_whitespace();
        if (word == "min" || word == "max") && self.eat('(') {
            let func = if word == "min" { Func::Min } else { Func::Max };
            let left = self.expr()?;
            self.skip_whitespace();
            if !self.eat(',') {
                return Err(self.not_a_number());
            }
            let right = self.expr()?;
            self.eat_closing_parenthesis()?;
            return Ok(Expr::Call {
                func,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        if (word == "high" || word == "low") && self.eat('(') {
            let keep = if word == "low" { Keep::Low } else { Keep::High };
            self.skip_whitespace();
            let id = self.ident()?;
            self.eat_closing_parenthesis()?;
            return Ok(Expr::Let {
                id,
                keep: Some(keep),
            });
        }
        self.pos = start;
        while self.peek().is_ascii_alphanumeric() || matches!(self.peek(), '_' | ':' | '.') {
            self.pos += 1;
        }
        let owner_and_key = &self.src[start..self.pos];
        if owner_and_key == word {
            return Ok(Expr::Let {
                id: word,
                keep: None,
            });
        }
        let Some((owner, key)) = owner_and_key.rsplit_once('.') else {
            return Err(self.not_a_number());
        };
        if !key.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_') || key.contains(':') {
            return Err(self.not_a_number());
        }
        let of = match grammar::Object::parse(owner)? {
            grammar::Object::Path(_) => {
                return Err(format!("'{owner}' is a path, which has no meters"))
            }
            of => of,
        };
        Ok(Expr::Meter {
            of,
            key: key.to_string(),
            spot: None,
        })
    }

    fn digits(&mut self) -> Result<u64, String> {
        let start = self.pos;
        if !self.peek().is_ascii_digit() {
            return Err(self.not_a_number());
        }
        while self.peek().is_ascii_digit() {
            self.pos += 1;
        }
        self.src[start..self.pos]
            .parse::<u64>()
            .map_err(|_| self.not_a_number())
    }

    fn ident(&mut self) -> Result<String, String> {
        let start = self.pos;
        let ch = self.peek();
        if !(ch.is_ascii_alphabetic() || ch == '_') {
            return Err(self.not_a_number());
        }
        self.pos += 1;
        while self.peek().is_ascii_alphanumeric() || self.peek() == '_' {
            self.pos += 1;
        }
        Ok(self.src[start..self.pos].to_string())
    }

    fn skip_whitespace(&mut self) {
        while self.peek().is_whitespace() {
            let ch = self.peek();
            self.pos += ch.len_utf8();
        }
    }

    /// Skip whitespace and eat the `)` that closes a group or a call, or refuse the expression.
    fn eat_closing_parenthesis(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        if self.eat(')') {
            Ok(())
        } else {
            Err(self.not_a_number())
        }
    }

    fn eat(&mut self, want: char) -> bool {
        if self.peek() == want {
            self.pos += want.len_utf8();
            true
        } else {
            false
        }
    }

    fn starts_with(&self, prefix: &str) -> bool {
        self.src[self.pos..].starts_with(prefix)
    }

    fn peek(&self) -> char {
        self.src[self.pos..].chars().next().unwrap_or('\0')
    }

    fn is_at_end(&self) -> bool {
        self.pos >= self.src.len()
    }

    fn not_a_number(&self) -> String {
        format!("expression '{}' is not a number", self.src)
    }
}
