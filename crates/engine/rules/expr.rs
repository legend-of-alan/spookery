//! Dice and arithmetic inside a rule number.

use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::{Intent, Target};
use crate::model::Now;
use crate::rule::{arith, Arith, Expr, Fun, Keep, Who};

pub struct Ctx<'a> {
    pub intent: &'a Intent,
}

/// Faces for one expression. `begin` starts a new roll seed. `note` records the shown line.
pub trait Dice {
    fn begin(&mut self);
    fn draw(&mut self, count: u32, sides: u32) -> Result<Vec<u32>, Reason>;
    fn note(&mut self, text: String);
}

/// No dice. A rule that rolls here is refused.
pub struct NoDice;

impl Dice for NoDice {
    fn begin(&mut self) {}

    fn draw(&mut self, _count: u32, _sides: u32) -> Result<Vec<u32>, Reason> {
        Err(Reason::Schema)
    }

    fn note(&mut self, _text: String) {}
}

pub struct EvalOutput {
    pub value: i64,
}

pub fn eval(canon: &Canon, now: &Now, ctx: &Ctx<'_>, expr: &Expr) -> Result<i64, Reason> {
    Ok(eval_with_dice(canon, now, ctx, expr, &mut NoDice)?.value)
}

/// Evaluate `expr`, drawing dice from `dice` and noting a line when any fall.
pub fn eval_with_dice(
    canon: &Canon,
    now: &Now,
    ctx: &Ctx<'_>,
    expr: &Expr,
    dice: &mut dyn Dice,
) -> Result<EvalOutput, Reason> {
    dice.begin();
    let step = eval_node(canon, now, ctx, expr, dice)?;
    if !step.dice.is_empty() {
        dice.note(format!("{} = {}", step.dice.join("; "), step.body));
    }
    Ok(EvalOutput { value: step.value })
}

struct EvalStep {
    value: i64,
    body: String,
    dice: Vec<String>,
}

fn eval_node(
    canon: &Canon,
    now: &Now,
    ctx: &Ctx<'_>,
    expr: &Expr,
    dice: &mut dyn Dice,
) -> Result<EvalStep, Reason> {
    match expr {
        Expr::Num(n) => Ok(EvalStep {
            value: *n,
            body: n.to_string(),
            dice: Vec::new(),
        }),
        Expr::Dice { n, sides, keep } => {
            let faces = dice.draw(*n, *sides)?;
            let value = keep_faces(&faces, *keep)?;
            let listed = faces
                .iter()
                .map(u32::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            Ok(EvalStep {
                value,
                body: value.to_string(),
                dice: vec![format!("{} → {listed}", name_dice(*n, *sides, *keep))],
            })
        }
        Expr::Meter { who, key, .. } => {
            let value = read_meter(canon, now, ctx, who, key)?;
            Ok(EvalStep {
                value,
                body: value.to_string(),
                dice: Vec::new(),
            })
        }
        Expr::Neg(inner) => {
            let inner = eval_node(canon, now, ctx, inner, dice)?;
            let value = inner.value.checked_neg().ok_or(Reason::OutOfRange)?;
            Ok(EvalStep {
                value,
                body: negate_body(&inner.body),
                dice: inner.dice,
            })
        }
        Expr::Bin { op, left, right } => {
            let left = eval_node(canon, now, ctx, left, dice)?;
            let right = eval_node(canon, now, ctx, right, dice)?;
            let value = arith(*op, left.value, right.value)?;
            let mut rolled = left.dice;
            rolled.extend(right.dice);
            Ok(EvalStep {
                value,
                body: format!("{} {} {}", left.body, name_op(*op), right.body),
                dice: rolled,
            })
        }
        Expr::Call { fun, left, right } => {
            let left = eval_node(canon, now, ctx, left, dice)?;
            let right = eval_node(canon, now, ctx, right, dice)?;
            let value = match fun {
                Fun::Min => left.value.min(right.value),
                Fun::Max => left.value.max(right.value),
            };
            let mut rolled = left.dice;
            rolled.extend(right.dice);
            let name = match fun {
                Fun::Min => "min",
                Fun::Max => "max",
            };
            Ok(EvalStep {
                value,
                body: format!("{name}({}, {})", left.body, right.body),
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

fn name_op(op: Arith) -> &'static str {
    match op {
        Arith::Add => "+",
        Arith::Sub => "-",
        Arith::Mul => "*",
        Arith::Div => "/",
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
    now: &Now,
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
            Target::Actor(token) | Target::Place(token) => Ok(token.clone()),
            _ => Err(Reason::Schema),
        };
    }
    Ok(id.to_string())
}

pub(crate) fn resolve_who_id(now: &Now, ctx: &Ctx<'_>, who: &Who) -> Result<String, Reason> {
    match who {
        Who::Self_ => Ok(ctx.intent.actor_id.clone()),
        Who::Target => match &ctx.intent.target {
            Target::Actor(id) | Target::Place(id) => Ok(id.clone()),
            _ => Err(Reason::Schema),
        },
        Who::Here => now
            .actors
            .get(&ctx.intent.actor_id)
            .map(|actor| actor.place().to_string())
            .ok_or(Reason::UnknownKey),
        Who::Actor(id) => Ok(id.clone()),
        Who::Place(id) => Ok(id.clone()),
        Who::World => Err(Reason::Schema),
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{eval_with_dice, Ctx, Dice};
    use crate::canon::Canon;
    use crate::error::Reason;
    use crate::intent::{ActorKind, Intent, IntentArgs, Source, Target};
    use crate::meter::{MeterDef, MeterLive};
    use crate::model::{ActCounters, ActorLive, Now};
    use crate::rule::{parse_expr, Keep};

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

        fn note(&mut self, text: String) {
            self.notes.push(text);
        }
    }

    fn intent(actor: &str, target: &str) -> Intent {
        Intent {
            id: "t".to_string(),
            round_index: 0,
            actor_id: actor.to_string(),
            actor_kind: ActorKind::Player,
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
            budget: crate::canon::tick_settings::Budget::defaults(),
            checkpoint_every: 100,
        }
    }

    fn now() -> Now {
        let mut meters = BTreeMap::new();
        meters.insert("str_mod".to_string(), MeterLive { base: 3, offset: 0 });
        Now {
            schema: "now".to_string(),
            shard_id: "t".to_string(),
            pack_rev: String::new(),
            rules_rev: 0,
            state_rev: 0,
            round_index: 0,
            world_time: 0,
            last_tick_at: String::new(),
            presence: crate::model::Presence::default(),
            pause: crate::model::PauseView {
                named_plot_clocks: crate::model::RunState::Running,
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
                    act: ActCounters::fresh(),
                },
            )]),
            clocks: BTreeMap::new(),
            messages: Vec::new(),
            flags: Vec::new(),
        }
    }

    #[test]
    fn damage_dice_show_faces_and_the_modifier() {
        let expr = parse_expr("2d6 + self.str_mod").unwrap();
        let intent = intent("ada", "ada");
        let mut dice = Script {
            faces: vec![4, 5],
            at: 0,
            notes: Vec::new(),
        };
        let evaluated =
            eval_with_dice(&canon(), &now(), &Ctx { intent: &intent }, &expr, &mut dice).unwrap();
        assert_eq!(evaluated.value, 12);
        assert_eq!(dice.notes, vec!["2d6 → 4, 5 = 9 + 3".to_string()]);
    }

    #[test]
    fn the_higher_face_is_kept() {
        let expr = parse_expr("2d20kh1").unwrap();
        let intent = intent("ada", "ada");
        let mut dice = Script {
            faces: vec![7, 18],
            at: 0,
            notes: Vec::new(),
        };
        let evaluated =
            eval_with_dice(&canon(), &now(), &Ctx { intent: &intent }, &expr, &mut dice).unwrap();
        assert_eq!(evaluated.value, 18);
        assert!(matches!(
            expr,
            crate::rule::Expr::Dice {
                keep: Some(Keep::High),
                ..
            }
        ));
    }

    #[test]
    fn overflow_is_an_error() {
        let expr = parse_expr("9223372036854775807 + 1").unwrap();
        let intent = intent("ada", "ada");
        let err = eval_with_dice(
            &canon(),
            &now(),
            &Ctx { intent: &intent },
            &expr,
            &mut super::NoDice,
        );
        assert_eq!(
            err.map(|evaluated| evaluated.value),
            Err(Reason::OutOfRange)
        );
    }
}
