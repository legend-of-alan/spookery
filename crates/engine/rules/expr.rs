//! Dice and arithmetic inside a rule number.

use crate::ecs::component;
use crate::ecs::id::ActorId;
use crate::entity;
use crate::intent::Target;
use crate::rules::context::Context;
use crate::rules::grammar;
use crate::rules::rule::LineColumn;
use crate::Reason;

/// Faces for one expression. `begin` starts a new roll seed. `push_line` records the shown line.
pub(crate) trait Dice {
    fn begin(&mut self);
    fn draw(&mut self, count: u32, sides: u32) -> Result<Vec<u32>, Reason>;
    fn push_line(&mut self, text: String);
}

/// No dice. A rule that rolls here is refused.
struct NoDice;

impl Dice for NoDice {
    fn begin(&mut self) {}

    fn draw(&mut self, _count: u32, _sides: u32) -> Result<Vec<u32>, Reason> {
        Err(Reason::Schema)
    }

    fn push_line(&mut self, _text: String) {}
}

pub(crate) fn eval(context: &Context, expr: &Expr) -> Result<i64, Reason> {
    eval_with_dice(context, expr, &mut NoDice)
}

/// Evaluate `expr`, drawing dice from `dice` and noting a line when any fall.
pub(crate) fn eval_with_dice(
    context: &Context,
    expr: &Expr,
    dice: &mut dyn Dice,
) -> Result<i64, Reason> {
    Ok(eval_with_label(context, expr, dice, "")?.value)
}

/// Roll a rule's `let:` in written order on `dice`, each number reading the ids above it. An
/// id whose number rolls writes its dice line with the id in front.
pub(crate) fn roll_lets(
    context: &Context,
    lets: &[(String, Expr)],
    dice: &mut dyn Dice,
) -> Result<Vec<LetValue>, Reason> {
    let mut values = Vec::new();
    for (id, expr) in lets {
        let step = eval_with_label(&context.bind(&values), expr, dice, &format!("{id}: "))?;
        values.push(LetValue {
            id: id.clone(),
            value: step.value,
            faces: step.faces,
        });
    }
    Ok(values)
}

/// Whether `word` can be a `let:` id: a bare word that a number reads back as that id, and
/// not an owner word such as `self` or `world` or a call such as `min` or `high`.
pub(crate) fn is_let_id(word: &str) -> bool {
    let read = Expr::Let {
        id: word.to_string(),
        keep: None,
    };
    let cast_id = matches!(
        grammar::Object::parse(word),
        Ok(grammar::Object::Actor(grammar::Actor::Id(_)))
    );
    let call = ["min", "max", "high", "low"].contains(&word);
    Expr::parse(word) == Ok(read) && cast_id && !call
}

/// One `let:` id as the rule rolled it: its value, and its faces when its number is one dice
/// group, for `high` and `low` to read.
pub(crate) struct LetValue {
    pub id: String,
    pub value: i64,
    pub faces: Vec<u32>,
}

/// Evaluate `expr` on a new roll seed, noting a line with `label` in front when any dice fall.
fn eval_with_label(
    context: &Context,
    expr: &Expr,
    dice: &mut dyn Dice,
    label: &str,
) -> Result<EvalStep, Reason> {
    dice.begin();
    let step = eval_node(context, expr, dice)?;
    if !step.dice.is_empty() {
        dice.push_line(format!("{label}{} = {}", step.dice.join("; "), step.text));
    }
    Ok(step)
}

struct EvalStep {
    value: i64,
    text: String,
    dice: Vec<String>,
    /// The faces, when the number is one dice group: what `high` and `low` of a `let:` read.
    faces: Vec<u32>,
}

impl From<i64> for EvalStep {
    /// A value that rolled no dice.
    fn from(value: i64) -> Self {
        EvalStep {
            value,
            text: value.to_string(),
            dice: Vec::new(),
            faces: Vec::new(),
        }
    }
}

fn eval_node(context: &Context, expr: &Expr, dice: &mut dyn Dice) -> Result<EvalStep, Reason> {
    match expr {
        Expr::Num(n) => Ok(EvalStep::from(*n)),
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
                faces,
            })
        }
        Expr::Meter { of, key, .. } => Ok(EvalStep::from(read_meter(context, of, key)?)),
        Expr::Let { id, keep } => {
            let stored = (context.lets.iter())
                .find(|stored| stored.id == *id)
                .ok_or(Reason::Schema)?;
            Ok(EvalStep::from(match keep {
                Some(_) => keep_faces(&stored.faces, *keep)?,
                None => stored.value,
            }))
        }
        Expr::Neg(inner) => {
            let inner = eval_node(context, inner, dice)?;
            let value = inner.value.checked_neg().ok_or(Reason::OutOfRange)?;
            Ok(EvalStep {
                value,
                text: negate_body(&inner.text),
                dice: inner.dice,
                faces: Vec::new(),
            })
        }
        Expr::Binary { op, left, right } => {
            let left = eval_node(context, left, dice)?;
            let right = eval_node(context, right, dice)?;
            let value = op.eval(left.value, right.value)?;
            let mut rolled = left.dice;
            rolled.extend(right.dice);
            Ok(EvalStep {
                value,
                text: format!("{} {} {}", left.text, name_op(*op), right.text),
                dice: rolled,
                faces: Vec::new(),
            })
        }
        Expr::Call { fun, left, right } => {
            let left = eval_node(context, left, dice)?;
            let right = eval_node(context, right, dice)?;
            let value = match fun {
                Func::Min => left.value.min(right.value),
                Func::Max => left.value.max(right.value),
            };
            let mut rolled = left.dice;
            rolled.extend(right.dice);
            let function = match fun {
                Func::Min => "min",
                Func::Max => "max",
            };
            Ok(EvalStep {
                value,
                text: format!("{function}({}, {})", left.text, right.text),
                dice: rolled,
                faces: Vec::new(),
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

fn read_meter(context: &Context, of: &grammar::Object, key: &str) -> Result<i64, Reason> {
    let id = resolve_entity(context, of)?;
    (context.world.entities().get::<component::Meter>(&id)).value(key)
}

/// `id: target` is the intent's target token. Any other id is itself.
pub(crate) fn resolve_item_id(context: &Context, id: &str) -> Result<String, Reason> {
    if id == "target" {
        return context
            .intent
            .target
            .token()
            .map(str::to_string)
            .ok_or(Reason::Schema);
    }
    Ok(id.to_string())
}

/// Who holds the meter, flag or item a rule names: `here` and `place:` are a place, `story:`
/// is a story, `world` is the world, and anyone else is an actor. An exit holds none of them:
/// its door is an `exit_open`, and a door's flag is read by `cond::holds`.
pub(crate) fn resolve_entity(
    context: &Context,
    of: &grammar::Object,
) -> Result<entity::Id, Reason> {
    match of {
        grammar::Object::Global => Ok(entity::Id::Global),
        grammar::Object::Story(id) => Ok(entity::Id::Story(id.clone())),
        grammar::Object::Exit(_) => Err(Reason::UnknownId),
        grammar::Object::Place(id) => Ok(entity::Id::Place(id.clone())),
        grammar::Object::Here => Ok(entity::Id::Place(resolve_here(context)?.into())),
        grammar::Object::Actor(actor) => {
            Ok(entity::Id::Actor(resolve_actor(context, actor)?.clone()))
        }
    }
}

/// The actor `actor` names: the one acting, the move's target, or an actor by id. A target
/// that is a place, an exit or an item names no actor, so it is `UNKNOWN_ID`.
pub(crate) fn resolve_actor<'a>(
    context: &Context<'_, 'a>,
    actor: &'a grammar::Actor,
) -> Result<&'a ActorId, Reason> {
    match actor {
        grammar::Actor::Self_ => context.intent.actor_id().ok_or(Reason::Schema),
        grammar::Actor::Target => match &context.intent.target {
            Target::Actor(id) => Ok(id),
            Target::Empty => Err(Reason::Schema),
            Target::Place(_) | Target::Exit(_) | Target::Item(_) => Err(Reason::UnknownId),
        },
        grammar::Actor::Id(id) => Ok(id),
    }
}

/// Where the acting actor stands: a place id, or an exit's address while they walk.
pub(crate) fn resolve_here<'a>(context: &Context<'_, 'a>) -> Result<&'a str, Reason> {
    let actor_id = context.intent.actor_id().ok_or(Reason::Schema)?;
    let place = context.world.actors().get::<component::Place>(actor_id);
    place.get().ok_or(Reason::UnknownId)
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
        fun: Func,
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
pub(crate) fn stamp_spots(expr: &mut Expr, line: u32, column: u32) {
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
        Expr::Num(_) | Expr::Dice { .. } | Expr::Let { .. } => {}
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
            self.eat_closing_parenthesis()?;
            return Ok(inner);
        }
        if self.peek().is_ascii_digit() {
            return self.number_or_dice();
        }
        if self.peek().is_ascii_alphabetic() || self.peek() == '_' {
            return self.call_or_read();
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

    fn call_or_read(&mut self) -> Result<Expr, String> {
        let start = self.pos;
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
            self.eat_closing_parenthesis()?;
            return Ok(Expr::Call {
                fun,
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
            return Err(self.error());
        };
        if !key.starts_with(|ch: char| ch.is_ascii_alphabetic() || ch == '_') || key.contains(':') {
            return Err(self.error());
        }
        let of = match grammar::Object::parse(owner)? {
            grammar::Object::Exit(_) => {
                return Err(format!("'{owner}' is an exit, which has no meters"))
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

    /// Skip whitespace and eat the `)` that closes a group or a call, or refuse the expression.
    fn eat_closing_parenthesis(&mut self) -> Result<(), String> {
        self.skip_whitespace();
        if self.eat(')') {
            Ok(())
        } else {
            Err(self.error())
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
