//! A number in a rule: its syntax tree and the walks over it. `parse` reads text into the tree
//! and `eval` reads the tree to a value, drawing its dice.

use crate::rules::grammar;
use crate::rules::rule::LineColumn;
use crate::Reason;

pub(crate) mod eval;
mod parse;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Keep {
    High,
    Low,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Func {
    Min,
    Max,
}

/// A number in a rule. Parsed once, when the pack loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Num(i64),
    Dice {
        count: u32,
        sides: u32,
        keep: Option<Keep>,
    },
    Meter {
        of: grammar::Object,
        key: String,
        spot: Option<LineColumn>,
    },
    Binary {
        op: BinOp,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Neg(Box<Expr>),
    Call {
        func: Func,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    /// A read of a rule's `let:` id: its value, or with `keep`, the highest or lowest face of
    /// its one dice group.
    Let {
        id: String,
        keep: Option<Keep>,
    },
}

impl BinOp {
    /// Checked arithmetic. Division rounds down. Overflow and a zero divisor are errors.
    pub(crate) fn eval(self, left: i64, right: i64) -> Result<i64, Reason> {
        match self {
            BinOp::Add => left.checked_add(right).ok_or(Reason::OutOfRange),
            BinOp::Sub => left.checked_sub(right).ok_or(Reason::OutOfRange),
            BinOp::Mul => left.checked_mul(right).ok_or(Reason::OutOfRange),
            BinOp::Div => floor_div(left, right),
        }
    }
}

impl Func {
    pub(crate) fn eval(self, left: i64, right: i64) -> i64 {
        match self {
            Func::Min => left.min(right),
            Func::Max => left.max(right),
        }
    }
}

fn floor_div(left: i64, right: i64) -> Result<i64, Reason> {
    if right == 0 {
        return Err(Reason::OutOfRange);
    }
    let quot = left.checked_div(right).ok_or(Reason::OutOfRange)?;
    let rem = left.checked_rem(right).ok_or(Reason::OutOfRange)?;
    if rem != 0 && (left < 0) != (right < 0) {
        quot.checked_sub(1).ok_or(Reason::OutOfRange)
    } else {
        Ok(quot)
    }
}

impl Expr {
    /// A value with no dice and no meter reads. Overflow is not a value.
    pub(crate) fn constant(&self) -> Option<i64> {
        match self {
            Expr::Num(n) => Some(*n),
            Expr::Dice { .. } | Expr::Meter { .. } | Expr::Let { .. } => None,
            Expr::Neg(inner) => inner.constant().and_then(|n| n.checked_neg()),
            Expr::Binary { op, left, right } => {
                let left = left.constant()?;
                let right = right.constant()?;
                op.eval(left, right).ok()
            }
            Expr::Call { func, left, right } => {
                let left = left.constant()?;
                let right = right.constant()?;
                Some(func.eval(left, right))
            }
        }
    }

    /// Whether the number rolls dice. A `let:` read counts, since its value was rolled.
    pub(crate) fn needs_dice(&self) -> bool {
        match self {
            Expr::Dice { .. } | Expr::Let { .. } => true,
            Expr::Neg(inner) => inner.needs_dice(),
            Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
                left.needs_dice() || right.needs_dice()
            }
            Expr::Num(_) | Expr::Meter { .. } => false,
        }
    }

    /// Every meter read and `let:` read in the number, left to right.
    pub(crate) fn reads(&self) -> Vec<&Expr> {
        match self {
            Expr::Meter { .. } | Expr::Let { .. } => vec![self],
            Expr::Neg(inner) => inner.reads(),
            Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
                let mut reads = left.reads();
                reads.extend(right.reads());
                reads
            }
            Expr::Num(_) | Expr::Dice { .. } => Vec::new(),
        }
    }
}

/// Fill each meter file location that the parser left empty. A structured meter keeps its own.
pub(crate) fn stamp_file_locations(expr: &mut Expr, line: u32, column: u32) {
    if line == 0 {
        return;
    }
    let spot = LineColumn { line, column };
    stamp_at(expr, spot);
}

fn stamp_at(expr: &mut Expr, spot: LineColumn) {
    match expr {
        Expr::Meter {
            spot: meter_spot, ..
        } => {
            meter_spot.get_or_insert(spot);
        }
        Expr::Neg(inner) => stamp_at(inner, spot),
        Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
            stamp_at(left, spot);
            stamp_at(right, spot);
        }
        Expr::Num(_) | Expr::Dice { .. } | Expr::Let { .. } => {}
    }
}

/// Whether `word` can be a `let:` id: a bare word that a number reads back as that id, and
/// not an owner word such as `self` or `world` or a call such as `min` or `high`.
pub(crate) fn is_let_id(word: &str) -> bool {
    let read = Expr::Let {
        id: word.to_string(),
        keep: None,
    };
    let is_cast_id = matches!(
        grammar::Object::parse(word),
        Ok(grammar::Object::Actor(grammar::Actor::Id(_)))
    );
    let is_call = ["min", "max", "high", "low"].contains(&word);
    Expr::parse(word) == Ok(read) && is_cast_id && !is_call
}
