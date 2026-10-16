use crate::canon::Canon;
use crate::error::Reason;
use crate::intent::{Intent, Target};
use crate::meter;
use crate::model::Now;
use crate::rule::{NumberExpr, Who};

pub struct Ctx<'a> {
    pub intent: &'a Intent,
}

pub fn eval(canon: &Canon, now: &Now, ctx: &Ctx<'_>, expr: &NumberExpr) -> Result<i64, Reason> {
    match expr {
        NumberExpr::Literal(n) => Ok(*n),
        NumberExpr::Meter { who, key, plus } => {
            let actor_id = actor_of(now, ctx, who)?;
            let def = canon.meters.get(key).ok_or(Reason::UnknownKey)?;
            let live = now
                .actors
                .get(&actor_id)
                .ok_or(Reason::UnknownKey)?
                .meters()
                .get(key)
                .ok_or(Reason::UnknownKey)?;
            Ok(meter::current(def, live)?.saturating_add(*plus))
        }
    }
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

pub(crate) fn actor_of(now: &Now, ctx: &Ctx<'_>, who: &Who) -> Result<String, Reason> {
    match who {
        Who::Self_ | Who::Subject => Ok(ctx.intent.actor_id.clone()),
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
