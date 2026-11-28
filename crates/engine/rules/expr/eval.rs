//! Reading a number to a value, drawing its dice and writing the dice line.

use crate::ecs::component;
use crate::rules::context::Context;
use crate::rules::expr::{BinOp, Expr, Func, Keep};
use crate::rules::grammar;
use crate::Reason;

/// Faces for one expression. `start` starts a new roll seed. `push_line` records the shown line.
pub(crate) trait Dice {
    fn start(&mut self);
    fn draw(&mut self, count: u32, sides: u32) -> Result<Vec<u32>, Reason>;
    fn push_line(&mut self, text: String);
}

/// No dice. A rule that rolls here is refused.
struct NoDice;

impl Dice for NoDice {
    fn start(&mut self) {}

    fn draw(&mut self, _count: u32, _sides: u32) -> Result<Vec<u32>, Reason> {
        Err(Reason::Schema)
    }

    fn push_line(&mut self, _text: String) {}
}

/// Evaluate `expr` with no dice. A number that rolls is refused.
pub(crate) fn eval(context: &Context, expr: &Expr) -> Result<i64, Reason> {
    roll_number(context, expr, &mut NoDice)
}

/// Roll `expr` on `dice`, noting a line when any fall.
pub(crate) fn roll_number(
    context: &Context,
    expr: &Expr,
    dice: &mut dyn Dice,
) -> Result<i64, Reason> {
    Ok(roll_with_label(context, expr, dice, "")?.value)
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
        let shown = roll_with_label(&context.bind(&values), expr, dice, &format!("{id}: "))?;
        values.push(LetValue {
            id: id.clone(),
            value: shown.value,
            faces: shown.faces,
        });
    }
    Ok(values)
}

/// One `let:` id as the rule rolled it: its value, and its faces when its number is one dice
/// group, for `high` and `low` to read.
pub(crate) struct LetValue {
    pub id: String,
    pub value: i64,
    pub faces: Vec<u32>,
}

/// Roll `expr` on a new roll seed, noting a line with `label` in front when any dice fall.
fn roll_with_label(
    context: &Context,
    expr: &Expr,
    dice: &mut dyn Dice,
    label: &str,
) -> Result<ShownValue, Reason> {
    dice.start();
    let shown = roll_node(context, expr, dice)?;
    if !shown.dice_lines.is_empty() {
        dice.push_line(format!(
            "{label}{} = {}",
            shown.dice_lines.join("; "),
            shown.text
        ));
    }
    Ok(shown)
}

/// A number's value and how its line shows it: `text` is the arithmetic with each part's value,
/// and `dice_lines` holds one line per dice group that fell.
struct ShownValue {
    value: i64,
    text: String,
    dice_lines: Vec<String>,
    /// The faces, when the number is one dice group: what `high` and `low` of a `let:` read.
    faces: Vec<u32>,
}

impl From<i64> for ShownValue {
    /// A value that rolled no dice.
    fn from(value: i64) -> Self {
        ShownValue {
            value,
            text: value.to_string(),
            dice_lines: Vec::new(),
            faces: Vec::new(),
        }
    }
}

fn roll_node(context: &Context, expr: &Expr, dice: &mut dyn Dice) -> Result<ShownValue, Reason> {
    match expr {
        Expr::Num(n) => Ok(ShownValue::from(*n)),
        Expr::Dice { count, sides, keep } => {
            let faces = dice.draw(*count, *sides)?;
            let value = keep_faces(&faces, *keep)?;
            let listed = faces
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Ok(ShownValue {
                value,
                text: value.to_string(),
                dice_lines: vec![format!("{} → {listed}", show_dice(*count, *sides, *keep))],
                faces,
            })
        }
        Expr::Meter { of, id, .. } => Ok(ShownValue::from(read_meter(context, of, id)?)),
        Expr::Let { id, keep } => {
            let stored = (context.lets.iter())
                .find(|stored| stored.id == *id)
                .ok_or(Reason::Schema)?;
            Ok(ShownValue::from(match keep {
                Some(_) => keep_faces(&stored.faces, *keep)?,
                None => stored.value,
            }))
        }
        Expr::Neg(inner) => {
            let inner = roll_node(context, inner, dice)?;
            let value = inner.value.checked_neg().ok_or(Reason::OutOfRange)?;
            Ok(ShownValue {
                value,
                text: show_neg(&inner.text),
                dice_lines: inner.dice_lines,
                faces: Vec::new(),
            })
        }
        Expr::Binary { op, left, right } => {
            let left = roll_node(context, left, dice)?;
            let right = roll_node(context, right, dice)?;
            let value = op.eval(left.value, right.value)?;
            let mut dice_lines = left.dice_lines;
            dice_lines.extend(right.dice_lines);
            Ok(ShownValue {
                value,
                text: format!("{} {} {}", left.text, show_op(*op), right.text),
                dice_lines,
                faces: Vec::new(),
            })
        }
        Expr::Call { func, left, right } => {
            let left = roll_node(context, left, dice)?;
            let right = roll_node(context, right, dice)?;
            let value = func.eval(left.value, right.value);
            let mut dice_lines = left.dice_lines;
            dice_lines.extend(right.dice_lines);
            Ok(ShownValue {
                value,
                text: format!("{}({}, {})", show_func(*func), left.text, right.text),
                dice_lines,
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

fn show_dice(count: u32, sides: u32, keep: Option<Keep>) -> String {
    let base = format!("{count}d{sides}");
    match keep {
        None => base,
        Some(Keep::High) => format!("{base}kh1"),
        Some(Keep::Low) => format!("{base}kl1"),
    }
}

fn show_func(func: Func) -> &'static str {
    match func {
        Func::Min => "min",
        Func::Max => "max",
    }
}

fn show_op(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
    }
}

fn show_neg(text: &str) -> String {
    let plain = text.chars().all(|ch| ch.is_ascii_digit());
    if plain {
        format!("-{text}")
    } else {
        format!("-({text})")
    }
}

fn read_meter(context: &Context, of: &grammar::Object, meter_id: &str) -> Result<i64, Reason> {
    let entity_id = of.entity_id(context)?;
    (context.world.entities().get::<component::Meter>(&entity_id)).value(meter_id)
}
