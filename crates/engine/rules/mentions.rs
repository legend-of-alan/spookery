//! What a rule's or an exit's lines name.

use std::collections::BTreeSet;

use crate::rules::cond::Cond;
use crate::rules::cost::Cost;
use crate::rules::expr::Expr;
use crate::rules::grammar;
use crate::rules::rule::{EffectTemplate, MoveTo, Rule, TargetKind};

/// What a rule's or exit's lines name, borrowed from those lines (`'l`) and sorted into entities,
/// item ids and `target` keys. The word `target` stands for whatever the verb targets, so it is
/// kept apart from the entities and items it could name.
#[derive(Default)]
pub(crate) struct Mentions<'l> {
    /// The entities other than `target`, in the order written.
    pub(crate) entities: Vec<grammar::Object>,
    /// The item ids other than `target`, each once.
    pub(crate) item_ids: BTreeSet<&'l str>,
    /// Each key that says `target`, as written and in the order written, with what the verb
    /// may target for it to name something. A refusal suggests the first.
    target_keys: Vec<(&'static str, &'static [TargetKind])>,
}

impl<'l> Mentions<'l> {
    /// What `conds` and `costs` name, and the entities `exprs` read, as `a number`: `subject:`,
    /// `id:` and `of:` on a condition, then a cost's `items:`.
    pub(crate) fn of(
        conds: &'l [Cond],
        costs: impl IntoIterator<Item = &'l Cost>,
        exprs: Vec<&Expr>,
    ) -> Self {
        let mut mentions = Mentions::default();
        for cond in conds {
            cond.visit(|cond| match cond {
                Cond::HasItem { subject, id, .. } | Cond::Wielding { subject, id } => {
                    mentions.add_actor("subject:", subject);
                    mentions.add_item_id("id:", id);
                }
                Cond::At { subject, .. } => mentions.add_actor("subject:", subject),
                Cond::Meter { of, .. } => mentions.add_entity("of:", of),
                Cond::Flag { of, .. } => mentions.add_flag_entity("of:", of),
                _ => {}
            });
        }
        for (id, _) in costs.into_iter().flat_map(|cost| &cost.items) {
            mentions.add_item_id("a cost's items:", id);
        }
        for read in exprs.into_iter().flat_map(Expr::reads) {
            if let Expr::Meter { of, .. } = read {
                mentions.add_entity("a number", of);
            }
        }
        mentions
    }

    fn add_entity(&mut self, key: &'static str, of: &grammar::Object) {
        match of {
            grammar::Object::Actor(actor) => self.add_actor(key, actor),
            of => self.entities.push(of.clone()),
        }
    }

    /// As `add_entity`, but `target` may also be an exit: a flag on its path.
    fn add_flag_entity(&mut self, key: &'static str, of: &grammar::Object) {
        match of {
            grammar::Object::Actor(grammar::Actor::Target) => {
                self.add_target(key, &[TargetKind::Actor, TargetKind::Path]);
            }
            of => self.add_entity(key, of),
        }
    }

    fn add_actor(&mut self, key: &'static str, actor: &grammar::Actor) {
        match actor {
            grammar::Actor::Target => self.add_target(key, &[TargetKind::Actor]),
            actor => self.entities.push(grammar::Object::Actor(actor.clone())),
        }
    }

    fn add_item_id(&mut self, key: &'static str, id: &'l str) {
        match id {
            "target" => self.add_target(key, &[TargetKind::Item]),
            id => _ = self.item_ids.insert(id),
        }
    }

    fn add_target(&mut self, key: &'static str, allowed: &'static [TargetKind]) {
        self.target_keys.push((key, allowed));
    }

    /// The first of the target keys where a verb that targets `target_kind` does not allow
    /// `target`, as the key and what it needs the verb to target. A habit, a change rule and an
    /// exit target nothing.
    pub(crate) fn find_misplaced_target(
        self,
        target_kind: TargetKind,
    ) -> Option<(&'static str, TargetKind)> {
        let misplaced =
            (self.target_keys.into_iter()).find(|(_, allowed)| !allowed.contains(&target_kind));
        misplaced.map(|(key, allowed)| (key, allowed[0]))
    }
}

impl Rule {
    /// The entities, item ids and `target`s the rule's lines name: its conditions', costs' and
    /// numbers', then on its effects `of:`, `subject:`, `to:`, a nested check's `by:`, `id:`, and
    /// `location to:` or `location via:` when it says `target`.
    pub(crate) fn mentions(&self) -> Mentions<'_> {
        let mut mentions = Mentions::of(&self.if_, self.costs(), self.exprs());
        self.visit_templates(|effect| {
            match effect {
                EffectTemplate::Meter { of, .. } | EffectTemplate::Item { of, .. } => {
                    mentions.add_entity("of:", of);
                }
                EffectTemplate::Flag { of, .. } => mentions.add_flag_entity("of:", of),
                EffectTemplate::Modifier { to, .. } | EffectTemplate::Memory { to, .. } => {
                    mentions.add_actor("to:", to);
                }
                EffectTemplate::Wield { subject, .. } | EffectTemplate::Unwield { subject } => {
                    mentions.add_actor("subject:", subject);
                }
                EffectTemplate::Check { by, .. } => mentions.add_actor("by:", by),
                EffectTemplate::Move { to, .. } => match to {
                    MoveTo::Target => mentions.add_target("location to:", &[TargetKind::Place]),
                    MoveTo::Via(via) if via == "target" => {
                        mentions.add_target("location via:", &[TargetKind::Path]);
                    }
                    MoveTo::Via(_) | MoveTo::Place(_) => {}
                },
                EffectTemplate::Note { .. } | EffectTemplate::Narrate { .. } => {}
            }
            if let EffectTemplate::Item { id, .. } | EffectTemplate::Wield { id, .. } = effect {
                mentions.add_item_id("id:", id);
            }
        });
        mentions
    }
}
