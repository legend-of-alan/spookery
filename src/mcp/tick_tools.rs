use spookery_kernel::model::ViewerId;
use spookery_kernel::perceive::{perceive_budget, Perceive};
use spookery_kernel::shard::Shard;
use spookery_kernel::Reason;

pub struct PerceiveIn {
    pub as_viewer: Option<String>,
    pub focus: Option<String>,
    pub budget_tokens: Option<u64>,
}

pub fn perceive(shard: &Shard, input: &PerceiveIn) -> Result<Perceive, Reason> {
    let as_arg = input.as_viewer.as_deref().ok_or(Reason::UnscopedPerceive)?;
    match ViewerId::parse(as_arg)? {
        ViewerId::Npc(_) => {}
        ViewerId::Player(_) => return Err(Reason::UnscopedPerceive),
    }
    match input.focus.as_deref() {
        None | Some("place" | "actor") => {}
        Some(_) => return Err(Reason::Schema),
    }
    let budget = input.budget_tokens.unwrap_or(800) as usize;
    perceive_budget(shard, as_arg, budget)
}
