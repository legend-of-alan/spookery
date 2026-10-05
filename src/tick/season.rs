use spookery_kernel::canon::Canon;
use spookery_kernel::ids::kernel_intent_id;
use spookery_kernel::intent::{ActorKind, Intent, IntentArgs, Source, Target};
use spookery_kernel::model::Now;
use spookery_kernel::moves::Audience;
use spookery_kernel::Reason;

/// The catch-up call site is the only resolver of `season_advance`.
/// A missing row is `UNKNOWN_MOVE`. No actor means no proposal is invented.
pub fn season_advance_intent(canon: &Canon, now: &Now) -> Result<Option<Intent>, Reason> {
    let Some(row) = canon
        .moves
        .get("season_advance")
        .and_then(|def| def.row.as_ref())
    else {
        return Err(Reason::UnknownMove);
    };
    if row.audience != Some(Audience::System) {
        return Err(Reason::UnknownMove);
    }
    let Some((actor_id, actor_kind)) = season_actor(now) else {
        return Ok(None);
    };
    Ok(Some(Intent {
        id: kernel_intent_id(&now.shard_id, now.tick_index, &actor_id, "season_advance"),
        tick_index: now.tick_index,
        actor_id,
        actor_kind,
        source: Source::System,
        move_name: "season_advance".to_string(),
        target: Target::Empty,
        args: IntentArgs::default(),
        rationale_code: String::new(),
    }))
}

fn season_actor(now: &Now) -> Option<(String, ActorKind)> {
    if let Some(id) = now
        .actors
        .iter()
        .find(|(_, actor)| actor.is_pc())
        .map(|(id, _)| id.clone())
    {
        return Some((id, ActorKind::Player));
    }
    let mut owners: Vec<String> = now
        .clocks
        .values()
        .flat_map(|clock| clock.owners.iter().cloned())
        .filter(|id| now.actors.get(id).is_some_and(|actor| actor.is_npc()))
        .collect();
    owners.sort();
    owners.dedup();
    owners.into_iter().next().map(|id| (id, ActorKind::Npc))
}
