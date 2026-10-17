//! One rule shape. The id is namespaced by the file that owns it.

use crate::error::Reason;
use crate::intent::Target;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub local_id: String,
    pub owner: Owner,
    pub when: When,
    pub if_: Vec<Cond>,
    pub roll: Option<RollSpec>,
    pub cost: Option<Cost>,
    pub do_: Vec<EffectTemplate>,
    pub success: Vec<EffectTemplate>,
    pub partial: Vec<EffectTemplate>,
    pub failure: Vec<EffectTemplate>,
    pub file: String,
    pub index: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
    Verb,
    Place(String),
    Item(String),
    Cast(String),
    Story(String),
}

impl Owner {
    pub fn rank(&self) -> u8 {
        match self {
            Owner::Verb => 0,
            Owner::Place(_) => 1,
            Owner::Item(_) => 2,
            Owner::Cast(_) => 3,
            Owner::Story(_) => 4,
        }
    }

    pub fn id(&self, local: &str) -> String {
        match self {
            Owner::Verb => format!("verb:{local}"),
            Owner::Place(owner) => format!("place:{owner}/{local}"),
            Owner::Item(owner) => format!("item:{owner}/{local}"),
            Owner::Cast(owner) => format!("cast:{owner}/{local}"),
            Owner::Story(owner) => format!("story:{owner}/{local}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum When {
    Verb { verb: String, target: TargetKind },
    Habit,
    Change(ChangeWatch),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    None,
    Actor,
    Place,
    Exit,
    Item,
}

impl TargetKind {
    pub fn word(self) -> &'static str {
        match self {
            TargetKind::None => "none",
            TargetKind::Actor => "actor",
            TargetKind::Place => "place",
            TargetKind::Exit => "exit",
            TargetKind::Item => "item",
        }
    }

    /// Exit names ride as places. Item ids ride as actors. Both are one typed token.
    pub fn store(self, token: &str) -> Target {
        match self {
            TargetKind::None => Target::Empty,
            TargetKind::Place | TargetKind::Exit => Target::Place(token.to_string()),
            TargetKind::Actor | TargetKind::Item => Target::Actor(token.to_string()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeWatch {
    Arrive { place: String, who: String },
    Leave { place: String, who: String },
    Meter { key: String, op: Op, bound: i64 },
    Flag { key: String, becomes: bool },
    Clock { id: String },
    Item { id: String, gained: bool },
    Knows { fact: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollSpec {
    pub check: String,
    pub target: Expr,
    pub partial_at: Option<Expr>,
}

/// Where a parsed meter read sits in the pack file. Line and column are 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spot {
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arith {
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
pub enum Fun {
    Min,
    Max,
}

/// A number in a rule. Parsed once, when the pack loads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Expr {
    Num(i64),
    Dice {
        n: u32,
        sides: u32,
        keep: Option<Keep>,
    },
    Meter {
        who: Who,
        key: String,
        spot: Option<Spot>,
    },
    Bin {
        op: Arith,
        left: Box<Expr>,
        right: Box<Expr>,
    },
    Neg(Box<Expr>),
    Call {
        fun: Fun,
        left: Box<Expr>,
        right: Box<Expr>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cost {
    pub minutes: u64,
    pub meters: Vec<(String, Expr)>,
    pub items: Vec<(String, Expr)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    HasItem {
        who: Who,
        id: String,
        count: u32,
    },
    Meter {
        who: Who,
        key: String,
        op: Op,
        value: Expr,
    },
    Flag {
        who: Who,
        key: String,
        is: bool,
    },
    At {
        who: Who,
        place: String,
    },
    Knows {
        who: Who,
        fact: String,
    },
    Clock {
        id: String,
        op: Op,
        value: Expr,
    },
    Hour {
        from: i64,
        to: i64,
    },
    Humans {
        op: Op,
        value: i64,
    },
    NoteExists(String),
    Wielding {
        who: Who,
        id: String,
    },
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Lt,
    Le,
    Eq,
    Ge,
    Gt,
}

impl Op {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "<" => Some(Op::Lt),
            "<=" => Some(Op::Le),
            "==" => Some(Op::Eq),
            ">=" => Some(Op::Ge),
            ">" => Some(Op::Gt),
            _ => None,
        }
    }

    pub fn holds(self, left: i64, right: i64) -> bool {
        match self {
            Op::Lt => left < right,
            Op::Le => left <= right,
            Op::Eq => left == right,
            Op::Ge => left >= right,
            Op::Gt => left > right,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Who {
    Self_,
    Target,
    Subject,
    Here,
    World,
    Place(String),
    Actor(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectTemplate {
    Meter {
        who: Who,
        key: String,
        add: Option<Expr>,
        set: Option<Expr>,
        resist: Option<ResistSpec>,
    },
    Flag {
        who: Who,
        key: String,
        value: bool,
        resist: Option<ResistSpec>,
    },
    Location {
        to: LocationTo,
        teleport: bool,
    },
    Note {
        id: String,
        to: Vec<String>,
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: NarrateTo,
        text: String,
        fact_ids: Vec<String>,
        resist: Option<ResistSpec>,
    },
    Clock {
        id: String,
        add: Option<i64>,
        start: bool,
    },
    Memory {
        who: Who,
        fact_ids: Vec<String>,
        text: Option<String>,
    },
    Item {
        who: Who,
        id: String,
        add: i64,
        max: Option<i64>,
        resist: Option<ResistSpec>,
    },
    Reveal {
        who: Who,
        fact: String,
    },
    Wield {
        who: Who,
        id: String,
    },
    Unwield {
        who: Who,
    },
}

impl EffectTemplate {
    pub fn resist(&self) -> Option<&ResistSpec> {
        match self {
            EffectTemplate::Meter { resist, .. }
            | EffectTemplate::Flag { resist, .. }
            | EffectTemplate::Narrate { resist, .. }
            | EffectTemplate::Item { resist, .. } => resist.as_ref(),
            _ => None,
        }
    }

    pub(crate) fn note_text(&self) -> Option<&str> {
        match self {
            EffectTemplate::Note { text, .. } | EffectTemplate::Narrate { text, .. } => Some(text),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationTo {
    Target,
    Place(String),
    Via(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NarrateTo {
    Here,
    Self_,
    Actors(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResistSpec {
    Same,
    Roll {
        who: Who,
        check: String,
        target: Expr,
        resisted: Vec<EffectTemplate>,
    },
}

#[derive(Clone, Debug)]
pub struct RuleTable {
    rules: Vec<Rule>,
}

impl RuleTable {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter()
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    pub fn is_verb(&self, name: &str) -> bool {
        self.rules
            .iter()
            .any(|rule| matches!(&rule.when, When::Verb { verb, .. } if verb == name))
    }

    pub fn verb(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| {
            matches!(&rule.owner, Owner::Verb)
                && matches!(&rule.when, When::Verb { verb, .. } if verb == name)
        })
    }

    pub fn named(&self, local: &str) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| rule.local_id == local || rule.id == local)
            .collect()
    }

    pub(crate) fn habits_of(&self, actor: &str) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| {
                matches!(&rule.owner, Owner::Cast(id) if id == actor)
                    && matches!(rule.when, When::Habit)
            })
            .collect()
    }

    /// Verbs the player can type, in file order.
    pub(crate) fn player_verbs(&self) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| matches!(rule.when, When::Verb { .. }))
            .collect()
    }

    pub fn ids_in_order(&self) -> Vec<&str> {
        let mut rules: Vec<&Rule> = self.rules.iter().collect();
        rules.sort_by(|left, right| {
            left.owner
                .rank()
                .cmp(&right.owner.rank())
                .then(left.file.cmp(&right.file))
                .then(left.index.cmp(&right.index))
        });
        rules.into_iter().map(|rule| rule.id.as_str()).collect()
    }
}

/// Checked arithmetic. Division rounds down. Overflow and a zero divisor are errors.
pub(crate) fn arith(op: Arith, left: i64, right: i64) -> Result<i64, Reason> {
    match op {
        Arith::Add => left.checked_add(right).ok_or(Reason::OutOfRange),
        Arith::Sub => left.checked_sub(right).ok_or(Reason::OutOfRange),
        Arith::Mul => left.checked_mul(right).ok_or(Reason::OutOfRange),
        Arith::Div => floor_div(left, right),
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
            Expr::Dice { .. } | Expr::Meter { .. } => None,
            Expr::Neg(inner) => inner.constant().and_then(|n| n.checked_neg()),
            Expr::Bin { op, left, right } => {
                let left = left.constant()?;
                let right = right.constant()?;
                arith(*op, left, right).ok()
            }
            Expr::Call { fun, left, right } => {
                let left = left.constant()?;
                let right = right.constant()?;
                Some(match fun {
                    Fun::Min => left.min(right),
                    Fun::Max => left.max(right),
                })
            }
        }
    }

    pub(crate) fn needs_dice(&self) -> bool {
        match self {
            Expr::Dice { .. } => true,
            Expr::Neg(inner) => inner.needs_dice(),
            Expr::Bin { left, right, .. } | Expr::Call { left, right, .. } => {
                left.needs_dice() || right.needs_dice()
            }
            Expr::Num(_) | Expr::Meter { .. } => false,
        }
    }

    pub(crate) fn meters<'a>(&'a self, out: &mut Vec<(&'a Who, &'a str, Option<Spot>)>) {
        match self {
            Expr::Meter { who, key, spot } => out.push((who, key, *spot)),
            Expr::Neg(inner) => inner.meters(out),
            Expr::Bin { left, right, .. } | Expr::Call { left, right, .. } => {
                left.meters(out);
                right.meters(out);
            }
            Expr::Num(_) | Expr::Dice { .. } => {}
        }
    }
}

/// Parse one expression. A literal zero divisor and `0dN` fail here.
pub(crate) fn parse_expr(src: &str) -> Result<Expr, String> {
    let mut parser = Parser { src, i: 0 };
    parser.skip();
    if parser.done() {
        return Err(parser.bad());
    }
    let expr = parser.expr()?;
    parser.skip();
    if !parser.done() {
        return Err(parser.bad());
    }
    if divides_by_zero(&expr) {
        return Err(format!("expression '{src}' divides by zero"));
    }
    Ok(expr)
}

/// Fill meter spots that the parser left empty. A structured meter keeps its own spot.
pub(crate) fn stamp_expr(expr: &mut Expr, line: u32, column: u32) {
    if line == 0 {
        return;
    }
    let spot = Spot { line, column };
    stamp_at(expr, spot);
}

fn stamp_at(expr: &mut Expr, spot: Spot) {
    match expr {
        Expr::Meter { spot: slot, .. } => {
            if slot.is_none() {
                *slot = Some(spot);
            }
        }
        Expr::Neg(inner) => stamp_at(inner, spot),
        Expr::Bin { left, right, .. } | Expr::Call { left, right, .. } => {
            stamp_at(left, spot);
            stamp_at(right, spot);
        }
        Expr::Num(_) | Expr::Dice { .. } => {}
    }
}

fn divides_by_zero(expr: &Expr) -> bool {
    match expr {
        Expr::Bin {
            op: Arith::Div,
            right,
            ..
        } if matches!(right.as_ref(), Expr::Num(0)) => true,
        Expr::Bin { left, right, .. } | Expr::Call { left, right, .. } => {
            divides_by_zero(left) || divides_by_zero(right)
        }
        Expr::Neg(inner) => divides_by_zero(inner),
        _ => false,
    }
}

struct Parser<'a> {
    src: &'a str,
    i: usize,
}

impl<'a> Parser<'a> {
    fn expr(&mut self) -> Result<Expr, String> {
        let mut left = self.term()?;
        loop {
            self.skip();
            let op = if self.eat('+') {
                Arith::Add
            } else if self.eat('-') {
                Arith::Sub
            } else {
                return Ok(left);
            };
            let right = self.term()?;
            left = Expr::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn term(&mut self) -> Result<Expr, String> {
        let mut left = self.unary()?;
        loop {
            self.skip();
            let op = if self.eat('*') {
                Arith::Mul
            } else if self.eat('/') {
                Arith::Div
            } else {
                return Ok(left);
            };
            let right = self.unary()?;
            left = Expr::Bin {
                op,
                left: Box::new(left),
                right: Box::new(right),
            };
        }
    }

    fn unary(&mut self) -> Result<Expr, String> {
        self.skip();
        if self.eat('-') {
            let inner = self.unary()?;
            return Ok(Expr::Neg(Box::new(inner)));
        }
        self.primary()
    }

    fn primary(&mut self) -> Result<Expr, String> {
        self.skip();
        if self.eat('(') {
            let inner = self.expr()?;
            self.skip();
            if !self.eat(')') {
                return Err(self.bad());
            }
            return Ok(inner);
        }
        if self.peek().is_ascii_digit() {
            return self.number_or_dice();
        }
        if self.peek().is_ascii_alphabetic() || self.peek() == '_' {
            return self.call_or_meter();
        }
        Err(self.bad())
    }

    fn number_or_dice(&mut self) -> Result<Expr, String> {
        let n = self.digits()?;
        if self.eat('d') {
            let sides = self.digits()?;
            let keep = self.keep()?;
            if n == 0 || sides == 0 || n > u64::from(u32::MAX) || sides > u64::from(u32::MAX) {
                return Err(format!("expression '{}' rolls no dice", self.src));
            }
            return Ok(Expr::Dice {
                n: n as u32,
                sides: sides as u32,
                keep,
            });
        }
        if n > i64::MAX as u64 {
            return Err(self.bad());
        }
        Ok(Expr::Num(n as i64))
    }

    fn keep(&mut self) -> Result<Option<Keep>, String> {
        if self.starts("kh1") {
            self.i += 3;
            Ok(Some(Keep::High))
        } else if self.starts("kl1") {
            self.i += 3;
            Ok(Some(Keep::Low))
        } else if self.peek() == 'k' {
            Err(self.bad())
        } else {
            Ok(None)
        }
    }

    fn call_or_meter(&mut self) -> Result<Expr, String> {
        let word = self.ident()?;
        self.skip();
        if (word == "min" || word == "max") && self.eat('(') {
            let fun = if word == "min" { Fun::Min } else { Fun::Max };
            let left = self.expr()?;
            self.skip();
            if !self.eat(',') {
                return Err(self.bad());
            }
            let right = self.expr()?;
            self.skip();
            if !self.eat(')') {
                return Err(self.bad());
            }
            return Ok(Expr::Call {
                fun,
                left: Box::new(left),
                right: Box::new(right),
            });
        }
        let who = if word == "place" && self.eat(':') {
            Who::Place(self.ident()?)
        } else {
            match word.as_str() {
                "self" => Who::Self_,
                "target" => Who::Target,
                "subject" => Who::Subject,
                "here" => Who::Here,
                "world" => Who::World,
                _ => Who::Actor(word),
            }
        };
        self.skip();
        if !self.eat('.') {
            return Err(self.bad());
        }
        let key = self.ident()?;
        Ok(Expr::Meter {
            who,
            key,
            spot: None,
        })
    }

    fn digits(&mut self) -> Result<u64, String> {
        let start = self.i;
        if !self.peek().is_ascii_digit() {
            return Err(self.bad());
        }
        while self.peek().is_ascii_digit() {
            self.i += 1;
        }
        self.src[start..self.i]
            .parse::<u64>()
            .map_err(|_| self.bad())
    }

    fn ident(&mut self) -> Result<String, String> {
        let start = self.i;
        let ch = self.peek();
        if !(ch.is_ascii_alphabetic() || ch == '_') {
            return Err(self.bad());
        }
        self.i += 1;
        while self.peek().is_ascii_alphanumeric() || self.peek() == '_' {
            self.i += 1;
        }
        Ok(self.src[start..self.i].to_string())
    }

    fn skip(&mut self) {
        while self.peek().is_whitespace() {
            let ch = self.peek();
            self.i += ch.len_utf8();
        }
    }

    fn eat(&mut self, want: char) -> bool {
        if self.peek() == want {
            self.i += want.len_utf8();
            true
        } else {
            false
        }
    }

    fn starts(&self, prefix: &str) -> bool {
        self.src[self.i..].starts_with(prefix)
    }

    fn peek(&self) -> char {
        self.src[self.i..].chars().next().unwrap_or('\0')
    }

    fn done(&self) -> bool {
        self.i >= self.src.len()
    }

    fn bad(&self) -> String {
        format!("expression '{}' is not a number", self.src)
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_expr, Arith, Expr, Fun, Keep, Who};

    #[test]
    fn integers_dice_and_keep_parse() {
        assert_eq!(parse_expr("12").unwrap().constant(), Some(12));
        assert_eq!(parse_expr("-4").unwrap().constant(), Some(-4));
        match parse_expr("2d20kh1").unwrap() {
            Expr::Dice {
                n: 2,
                sides: 20,
                keep: Some(Keep::High),
            } => {}
            other => panic!("{other:?}"),
        }
        match parse_expr("2d20kl1").unwrap() {
            Expr::Dice {
                keep: Some(Keep::Low),
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn arithmetic_meters_and_calls_parse() {
        let expr = parse_expr("-(2d6 + self.str_mod)").unwrap();
        match expr {
            Expr::Neg(inner) => match *inner {
                Expr::Bin { op: Arith::Add, .. } => {}
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        match parse_expr("place:loft.light").unwrap() {
            Expr::Meter {
                who: Who::Place(id),
                key,
                ..
            } => {
                assert_eq!(id, "loft");
                assert_eq!(key, "light");
            }
            other => panic!("{other:?}"),
        }
        match parse_expr("min(self.hp, 4)").unwrap() {
            Expr::Call { fun: Fun::Min, .. } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(parse_expr("(10 + 2) / 3").unwrap().constant(), Some(4));
        assert_eq!(parse_expr("-5 / 2").unwrap().constant(), Some(-3));
    }

    #[test]
    fn zero_dice_and_a_literal_zero_divisor_fail() {
        let zero = parse_expr("0d6").unwrap_err();
        assert!(zero.contains("rolls no dice"), "{zero}");
        let div = parse_expr("10 / 0").unwrap_err();
        assert!(div.contains("divides by zero"), "{div}");
        let typo = parse_expr("2d6 +").unwrap_err();
        assert!(typo.contains("is not a number"), "{typo}");
    }
}
