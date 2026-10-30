//! Dice and arithmetic inside a rule number.

use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::{Intent, Target};
use crate::model::{Presence, State};
use crate::rule::{LineColumn, Who};
use crate::rules::roll::Outcome;

pub struct Ctx<'a> {
    pub intent: &'a Intent,
    /// Who is seated, for the `players:` condition and NPC habits.
    pub presence: &'a Presence,
}

/// Faces for one expression. `begin` starts a new roll seed. `note` records the shown line.
pub trait Dice {
    fn begin(&mut self);
    fn draw(&mut self, count: u32, sides: u32) -> Result<Vec<u32>, Reason>;
    fn log(&mut self, text: String);
    /// The band of `actor_id`'s resist roll against `target`.
    fn resist(
        &mut self,
        canon: &Canon,
        now: &State,
        actor_id: &str,
        check_id: &str,
        target: i64,
    ) -> Result<Outcome, Reason>;
}

/// No dice. A rule that rolls here is refused.
pub struct NoDice;

impl Dice for NoDice {
    fn begin(&mut self) {}

    fn draw(&mut self, _count: u32, _sides: u32) -> Result<Vec<u32>, Reason> {
        Err(Reason::Schema)
    }

    fn log(&mut self, _text: String) {}

    fn resist(
        &mut self,
        _: &Canon,
        _: &State,
        _: &str,
        _: &str,
        _: i64,
    ) -> Result<Outcome, Reason> {
        Err(Reason::Schema)
    }
}

pub struct EvalOutput {
    pub value: i64,
}

pub fn eval(canon: &Canon, now: &State, ctx: &Ctx<'_>, expr: &Expr) -> Result<i64, Reason> {
    Ok(eval_with_dice(canon, now, ctx, expr, &mut NoDice)?.value)
}

/// Evaluate `expr`, drawing dice from `dice` and noting a line when any fall.
pub fn eval_with_dice(
    canon: &Canon,
    now: &State,
    ctx: &Ctx<'_>,
    expr: &Expr,
    dice: &mut dyn Dice,
) -> Result<EvalOutput, Reason> {
    dice.begin();
    let step = eval_node(canon, now, ctx, expr, dice)?;
    if !step.dice.is_empty() {
        dice.log(format!("{} = {}", step.dice.join("; "), step.text));
    }
    Ok(EvalOutput { value: step.value })
}

struct EvalStep {
    value: i64,
    text: String,
    dice: Vec<String>,
}

fn eval_node(
    canon: &Canon,
    now: &State,
    ctx: &Ctx<'_>,
    expr: &Expr,
    dice: &mut dyn Dice,
) -> Result<EvalStep, Reason> {
    match expr {
        Expr::Num(n) => Ok(EvalStep {
            value: *n,
            text: n.to_string(),
            dice: Vec::new(),
        }),
        Expr::Dice {
            count: n,
            sides,
            keep,
        } => {
            let faces = dice.draw(*n, *sides)?;
            let value = keep_faces(&faces, *keep)?;
            let listed = faces
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Ok(EvalStep {
                value,
                text: value.to_string(),
                dice: vec![format!("{} → {listed}", name_dice(*n, *sides, *keep))],
            })
        }
        Expr::Meter { who, key, .. } => {
            let value = read_meter(canon, now, ctx, who, key)?;
            Ok(EvalStep {
                value,
                text: value.to_string(),
                dice: Vec::new(),
            })
        }
        Expr::Neg(inner) => {
            let inner = eval_node(canon, now, ctx, inner, dice)?;
            let value = inner.value.checked_neg().ok_or(Reason::OutOfRange)?;
            Ok(EvalStep {
                value,
                text: negate_body(&inner.text),
                dice: inner.dice,
            })
        }
        Expr::Binary { op, left, right } => {
            let left = eval_node(canon, now, ctx, left, dice)?;
            let right = eval_node(canon, now, ctx, right, dice)?;
            let value = op.eval(left.value, right.value)?;
            let mut rolled = left.dice;
            rolled.extend(right.dice);
            Ok(EvalStep {
                value,
                text: format!("{} {} {}", left.text, name_op(*op), right.text),
                dice: rolled,
            })
        }
        Expr::Call { fun, left, right } => {
            let left = eval_node(canon, now, ctx, left, dice)?;
            let right = eval_node(canon, now, ctx, right, dice)?;
            let value = match fun {
                Func::Min => left.value.min(right.value),
                Func::Max => left.value.max(right.value),
            };
            let mut rolled = left.dice;
            rolled.extend(right.dice);
            let name = match fun {
                Func::Min => "min",
                Func::Max => "max",
            };
            Ok(EvalStep {
                value,
                text: format!("{name}({}, {})", left.text, right.text),
                dice: rolled,
            })
        }
    }
}

fn keep_faces(faces: &[u32], keep: Option<Keep>) -> Result<i64, Reason> {
    let picked: i64 = match keep {
        Some(Keep::High) => i64::from(*faces.iter().max().ok_or(Reason::Schema)?),
        Some(Keep::Low) => i64::from(*faces.iter().min().ok_or(Reason::Schema)?),
        None => faces.iter().try_fold(0i64, |sum, face| {
            sum.checked_add(i64::from(*face)).ok_or(Reason::OutOfRange)
        })?,
    };
    Ok(picked)
}

fn name_dice(n: u32, sides: u32, keep: Option<Keep>) -> String {
    let base = format!("{n}d{sides}");
    match keep {
        None => base,
        Some(Keep::High) => format!("{base}kh1"),
        Some(Keep::Low) => format!("{base}kl1"),
    }
}

fn name_op(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
    }
}

fn negate_body(body: &str) -> String {
    let plain = body.chars().all(|ch| ch.is_ascii_digit());
    if plain {
        format!("-{body}")
    } else {
        format!("-({body})")
    }
}

fn read_meter(
    canon: &Canon,
    now: &State,
    ctx: &Ctx<'_>,
    who: &Who,
    key: &str,
) -> Result<i64, Reason> {
    let actor_id = resolve_who_id(now, ctx, who)?;
    now.meter_value(canon, &actor_id, key)
}

/// `id: target` is the intent's target token. Any other id is itself.
pub(crate) fn resolve_item_id(intent: &Intent, id: &str) -> Result<String, Reason> {
    if id == "target" {
        return match &intent.target {
            Target::Actor(token)
            | Target::Item(token)
            | Target::Place(token)
            | Target::Exit(token) => Ok(token.clone()),
            _ => Err(Reason::Schema),
        };
    }
    Ok(id.to_string())
}

pub(crate) fn resolve_who_id(now: &State, ctx: &Ctx<'_>, who: &Who) -> Result<String, Reason> {
    match who {
        Who::Self_ => Ok(ctx.intent.actor.actor_id().to_string()),
        Who::Target => match &ctx.intent.target {
            Target::Actor(id) | Target::Item(id) | Target::Place(id) | Target::Exit(id) => {
                Ok(id.clone())
            }
            _ => Err(Reason::Schema),
        },
        Who::Here => now
            .actors
            .get(ctx.intent.actor.actor_id())
            .map(|actor| actor.place().to_string())
            .ok_or(Reason::UnknownId),
        Who::Actor(id) => Ok(id.clone()),
        Who::Place(id) => Ok(id.clone()),
        Who::World => Err(Reason::Schema),
    }
}

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
        who: Who,
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
        fun: Func,
        left: Box<Expr>,
        right: Box<Expr>,
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
            Expr::Binary { op, left, right } => {
                let left = left.constant()?;
                let right = right.constant()?;
                op.eval(left, right).ok()
            }
            Expr::Call { fun, left, right } => {
                let left = left.constant()?;
                let right = right.constant()?;
                Some(match fun {
                    Func::Min => left.min(right),
                    Func::Max => left.max(right),
                })
            }
        }
    }

    pub(crate) fn needs_dice(&self) -> bool {
        match self {
            Expr::Dice { .. } => true,
            Expr::Neg(inner) => inner.needs_dice(),
            Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
                left.needs_dice() || right.needs_dice()
            }
            Expr::Num(_) | Expr::Meter { .. } => false,
        }
    }

    pub(crate) fn meter_reads(&self) -> Vec<(&Who, &str, Option<LineColumn>)> {
        match self {
            Expr::Meter { who, key, spot } => vec![(who, key, *spot)],
            Expr::Neg(inner) => inner.meter_reads(),
            Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
                let mut reads = left.meter_reads();
                reads.extend(right.meter_reads());
                reads
            }
            Expr::Num(_) | Expr::Dice { .. } => Vec::new(),
        }
    }

    /// Parse one expression. A literal zero divisor and `0dN` fail here.
    pub(crate) fn parse(src: &str) -> Result<Expr, String> {
        let mut parser = Parser { src, pos: 0 };
        parser.skip_whitespace();
        if parser.is_at_end() {
            return Err(parser.error());
        }
        let expr = parser.expr()?;
        parser.skip_whitespace();
        if !parser.is_at_end() {
            return Err(parser.error());
        }
        if divides_by_zero(&expr) {
            return Err(format!("expression '{src}' divides by zero"));
        }
        Ok(expr)
    }
}

/// Fill meter spots that the parser left empty. A structured meter keeps its own spot.
pub(crate) fn stamp_expr(expr: &mut Expr, line: u32, column: u32) {
    if line == 0 {
        return;
    }
    let spot = LineColumn { line, column };
    stamp_at(expr, spot);
}

fn stamp_at(expr: &mut Expr, spot: LineColumn) {
    match expr {
        Expr::Meter { spot: slot, .. } => {
            if slot.is_none() {
                *slot = Some(spot);
            }
        }
        Expr::Neg(inner) => stamp_at(inner, spot),
        Expr::Binary { left, right, .. } | Expr::Call { left, right, .. } => {
            stamp_at(left, spot);
            stamp_at(right, spot);
        }
        Expr::Num(_) | Expr::Dice { .. } => {}
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

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl<'a> Parser<'a> {
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
            self.skip_whitespace();
            if !self.eat(')') {
                return Err(self.error());
            }
            return Ok(inner);
        }
        if self.peek().is_ascii_digit() {
            return self.number_or_dice();
        }
        if self.peek().is_ascii_alphabetic() || self.peek() == '_' {
            return self.call_or_meter();
        }
        Err(self.error())
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
            return Err(self.error());
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
            Err(self.error())
        } else {
            Ok(None)
        }
    }

    fn call_or_meter(&mut self) -> Result<Expr, String> {
        let word = self.ident()?;
        self.skip_whitespace();
        if (word == "min" || word == "max") && self.eat('(') {
            let fun = if word == "min" { Func::Min } else { Func::Max };
            let left = self.expr()?;
            self.skip_whitespace();
            if !self.eat(',') {
                return Err(self.error());
            }
            let right = self.expr()?;
            self.skip_whitespace();
            if !self.eat(')') {
                return Err(self.error());
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
                "here" => Who::Here,
                "world" => Who::World,
                _ => Who::Actor(word),
            }
        };
        self.skip_whitespace();
        if !self.eat('.') {
            return Err(self.error());
        }
        let key = self.ident()?;
        Ok(Expr::Meter {
            who,
            key,
            spot: None,
        })
    }

    fn digits(&mut self) -> Result<u64, String> {
        let start = self.pos;
        if !self.peek().is_ascii_digit() {
            return Err(self.error());
        }
        while self.peek().is_ascii_digit() {
            self.pos += 1;
        }
        self.src[start..self.pos]
            .parse::<u64>()
            .map_err(|_| self.error())
    }

    fn ident(&mut self) -> Result<String, String> {
        let start = self.pos;
        let ch = self.peek();
        if !(ch.is_ascii_alphabetic() || ch == '_') {
            return Err(self.error());
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

    fn error(&self) -> String {
        format!("expression '{}' is not a number", self.src)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{eval_with_dice, BinOp, Ctx, Dice, Expr, Func, Keep};
    use crate::canon::Canon;
    use crate::error::Reason;
    use crate::intent::{Intent, IntentArgs, Source, Target, Viewer};
    use crate::meter::{MeterDef, MeterLive};
    use crate::model::{Act, ActorLive, Presence, State};
    use crate::rule::Who;

    static EMPTY: Presence = Presence {
        players: 0,
        actors_present: Vec::new(),
    };

    struct Script {
        faces: Vec<u32>,
        at: usize,
        notes: Vec<String>,
    }

    impl Dice for Script {
        fn begin(&mut self) {}

        fn draw(&mut self, count: u32, _sides: u32) -> Result<Vec<u32>, Reason> {
            let mut out = Vec::new();
            for _ in 0..count {
                let face = *self.faces.get(self.at).ok_or(Reason::Schema)?;
                self.at += 1;
                out.push(face);
            }
            Ok(out)
        }

        fn log(&mut self, text: String) {
            self.notes.push(text);
        }

        fn resist(
            &mut self,
            _: &Canon,
            _: &State,
            _: &str,
            _: &str,
            _: i64,
        ) -> Result<crate::rules::roll::Outcome, Reason> {
            Err(Reason::Schema)
        }
    }

    fn ctx(intent: &Intent) -> Ctx<'_> {
        Ctx {
            intent,
            presence: &EMPTY,
        }
    }

    fn intent(actor: &str, target: &str) -> Intent {
        Intent {
            id: "t".to_string(),
            round_index: 0,
            actor: Viewer::Player(actor.to_string()),
            source: Source::Player,
            move_name: "hit".to_string(),
            target: Target::Actor(target.to_string()),
            args: IntentArgs::default(),
            rationale_code: String::new(),
        }
    }

    fn canon() -> Canon {
        Canon {
            pack_id: "t".to_string(),
            title: "t".to_string(),
            system: String::new(),
            start_place: "room".to_string(),
            start_pc: "ada".to_string(),
            places: BTreeMap::new(),
            cast: BTreeMap::new(),
            facts: BTreeMap::new(),
            rules: crate::rule::RuleTable::new(Vec::new()),
            checks: BTreeMap::new(),
            meters: BTreeMap::from([(
                "str_mod".to_string(),
                MeterDef {
                    min: -5,
                    base_cap: 10,
                    max: 10,
                    start: 0,
                    hidden: false,
                },
            )]),
            items: BTreeMap::new(),
            budget: crate::canon::Budget::default(),
            checkpoint_every: 100,
        }
    }

    fn now() -> State {
        let mut meters = BTreeMap::new();
        meters.insert("str_mod".to_string(), MeterLive { base: 3, offset: 0 });
        State {
            schema: "save".to_string(),
            world_id: "t".to_string(),
            pack_rev: String::new(),
            rules_rev: 0,
            state_rev: 0,
            round_index: 0,
            world_time: 0,
            last_round_at: String::new(),
            pause: crate::model::Pause {
                presence_clocks: crate::model::RunState::Running,
            },
            places: BTreeMap::new(),
            actors: BTreeMap::from([(
                "ada".to_string(),
                ActorLive::Pc {
                    place: "room".to_string(),
                    flags: Vec::new(),
                    meters,
                    inventory: BTreeMap::new(),
                    wielded: None,
                    known_facts: Vec::new(),
                    visited: Vec::new(),
                    act: Act::default(),
                },
            )]),
            clocks: BTreeMap::new(),
            notes: Vec::new(),
            flags: Vec::new(),
        }
    }

    #[test]
    fn damage_dice_show_faces_and_the_modifier() {
        let expr = Expr::parse("2d6 + self.str_mod").unwrap();
        let intent = intent("ada", "ada");
        let mut dice = Script {
            faces: vec![4, 5],
            at: 0,
            notes: Vec::new(),
        };
        let evaluated = eval_with_dice(&canon(), &now(), &ctx(&intent), &expr, &mut dice).unwrap();
        assert_eq!(evaluated.value, 12);
        assert_eq!(dice.notes, vec!["2d6 → 4, 5 = 9 + 3".to_string()]);
    }

    #[test]
    fn the_higher_face_is_kept() {
        let expr = Expr::parse("2d20kh1").unwrap();
        let intent = intent("ada", "ada");
        let mut dice = Script {
            faces: vec![7, 18],
            at: 0,
            notes: Vec::new(),
        };
        let evaluated = eval_with_dice(&canon(), &now(), &ctx(&intent), &expr, &mut dice).unwrap();
        assert_eq!(evaluated.value, 18);
        assert!(matches!(
            expr,
            Expr::Dice {
                keep: Some(Keep::High),
                ..
            }
        ));
    }

    #[test]
    fn overflow_is_an_error() {
        let expr = Expr::parse("9223372036854775807 + 1").unwrap();
        let intent = intent("ada", "ada");
        let err = eval_with_dice(&canon(), &now(), &ctx(&intent), &expr, &mut super::NoDice);
        assert_eq!(
            err.map(|evaluated| evaluated.value),
            Err(Reason::OutOfRange)
        );
    }

    #[test]
    fn integers_dice_and_keep_parse() {
        assert_eq!(Expr::parse("12").unwrap().constant(), Some(12));
        assert_eq!(Expr::parse("-4").unwrap().constant(), Some(-4));
        match Expr::parse("2d20kh1").unwrap() {
            Expr::Dice {
                count: 2,
                sides: 20,
                keep: Some(Keep::High),
            } => {}
            other => panic!("{other:?}"),
        }
        match Expr::parse("2d20kl1").unwrap() {
            Expr::Dice {
                keep: Some(Keep::Low),
                ..
            } => {}
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn arithmetic_meters_and_calls_parse() {
        let expr = Expr::parse("-(2d6 + self.str_mod)").unwrap();
        match expr {
            Expr::Neg(inner) => match *inner {
                Expr::Binary { op: BinOp::Add, .. } => {}
                other => panic!("{other:?}"),
            },
            other => panic!("{other:?}"),
        }
        match Expr::parse("place:loft.light").unwrap() {
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
        match Expr::parse("min(self.hp, 4)").unwrap() {
            Expr::Call { fun: Func::Min, .. } => {}
            other => panic!("{other:?}"),
        }
        assert_eq!(Expr::parse("(10 + 2) / 3").unwrap().constant(), Some(4));
        assert_eq!(Expr::parse("-5 / 2").unwrap().constant(), Some(-3));
    }

    #[test]
    fn zero_dice_and_a_literal_zero_divisor_fail() {
        let zero = Expr::parse("0d6").unwrap_err();
        assert!(zero.contains("rolls no dice"), "{zero}");
        let div = Expr::parse("10 / 0").unwrap_err();
        assert!(div.contains("divides by zero"), "{div}");
        let typo = Expr::parse("2d6 +").unwrap_err();
        assert!(typo.contains("is not a number"), "{typo}");
    }
}
