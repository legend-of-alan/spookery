//! One rule shape. The id is namespaced by the file that owns it.

use std::collections::BTreeSet;

use crate::ecs::id::{ActorId, PlaceId};
use crate::rules::context::Context;
use crate::rules::expr::{self, Expr};
use crate::rules::grammar;
use crate::rules::roll::{classify, Roll};
use crate::Reason;

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    pub id: String,
    pub local_id: String,
    pub owner: Owner,
    pub when: When,
    pub if_: Vec<Cond>,
    /// `let:`: each id and the number it stores, in the order written. They roll first, and
    /// the rule's check, effects and later ids read them by id.
    pub lets: Vec<(String, Expr)>,
    pub check: Option<Check>,
    /// Paid when the action starts. Its `cost` is the rule's `cost`, whose ticks are how long
    /// the action takes.
    pub start: Phase,
    /// Paid when the action finishes.
    pub finish: Phase,
    /// Ticks the actor stays busy after the finish.
    pub cooldown: u64,
    /// Whether the actor may cancel the action while it is under way.
    pub cancellable: bool,
    pub do_: Vec<EffectTemplate>,
    pub success: Vec<EffectTemplate>,
    pub partial: Vec<EffectTemplate>,
    pub miss: Vec<EffectTemplate>,
    pub file: String,
    pub index: usize,
}

impl Rule {
    pub(crate) fn watched_place<'a>(&'a self, place: &'a str) -> &'a str {
        if place == "here" {
            if let Owner::Place(id) = &self.owner {
                return id;
            }
        }
        place
    }

    /// The start cost, then the finish cost.
    pub(crate) fn costs(&self) -> impl Iterator<Item = &Cost> {
        self.start.cost.iter().chain(&self.finish.cost)
    }

    /// Every effect the rule holds, in the order `visit_templates` walks them, with the effects
    /// in its nested checks' lists.
    pub(crate) fn templates(&self) -> Vec<&EffectTemplate> {
        let mut templates = Vec::new();
        self.visit_templates(|effect| templates.push(effect));
        templates
    }

    pub(crate) fn visit_templates<'a>(&'a self, mut visit: impl FnMut(&'a EffectTemplate)) {
        let lists = [&self.do_, &self.success, &self.partial, &self.miss];
        for effect in lists.into_iter().flatten() {
            effect.visit(&mut visit);
        }
    }

    /// Every number the rule holds: its `let:` numbers first, in the order written, then its
    /// check's total, success_at and partial_at, its costs, its conditions' numbers, and its
    /// effects' amounts, caps, ticks and nested checks.
    pub(crate) fn exprs(&self) -> Vec<&Expr> {
        let mut exprs: Vec<&Expr> = self.lets.iter().map(|(_, expr)| expr).collect();
        exprs.extend(self.check.iter().flat_map(Check::exprs));
        exprs.extend(self.costs().flat_map(Cost::exprs));
        exprs.extend(self.if_.iter().flat_map(Cond::exprs));
        self.visit_templates(|effect| match effect {
            EffectTemplate::Modifier { add, ticks, .. } => exprs.extend([add, ticks]),
            EffectTemplate::Meter { add, set, .. } => exprs.extend(add.iter().chain(set)),
            EffectTemplate::Item { add, max, .. } => exprs.extend([add].into_iter().chain(max)),
            EffectTemplate::Check { check, .. } => exprs.extend(check.exprs()),
            _ => {}
        });
        exprs
    }

    /// The owners, item ids and `target`s the rule's lines name: its conditions', costs' and
    /// numbers', then on its effects `of:`, `subject:`, `to:`, a nested check's `by:`, `id:`, and
    /// `location to:` or `location via:` when it says `target`.
    pub(crate) fn mentions(&self) -> Mentions<'_> {
        let mut mentions = Mentions::of_lines(&self.if_, self.costs(), self.exprs());
        self.visit_templates(|effect| {
            match effect {
                EffectTemplate::Meter { of, .. } | EffectTemplate::Item { of, .. } => {
                    mentions.add_owner("of:", of);
                }
                EffectTemplate::Flag { of, .. } => mentions.add_flag_owner("of:", of),
                EffectTemplate::Modifier { to, .. } | EffectTemplate::Memory { to, .. } => {
                    mentions.add_actor("to:", to);
                }
                EffectTemplate::Wield { subject, .. } | EffectTemplate::Unwield { subject } => {
                    mentions.add_actor("subject:", subject);
                }
                EffectTemplate::Check { by, .. } => mentions.add_actor("by:", by),
                EffectTemplate::Location { to, .. } => match to {
                    LocationTo::Target => mentions.add_target("location to:", &[TargetKind::Place]),
                    LocationTo::Via(via) if via == "target" => {
                        mentions.add_target("location via:", &[TargetKind::Path]);
                    }
                    LocationTo::Via(_) | LocationTo::Place(_) => {}
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

/// What a rule's or exit's lines name, borrowed from those lines (`'l`) and sorted by kind. The
/// word `target` stands for whatever the move targets, so it is kept apart from the owners and
/// items it could name.
#[derive(Default)]
pub(crate) struct Mentions<'l> {
    /// The owners other than `target`, in the order written.
    pub(crate) owners: Vec<grammar::Object>,
    /// The item ids other than `target`, each once.
    pub(crate) item_ids: BTreeSet<&'l str>,
    /// Each key that says `target`, as written and in the order written, with what the move
    /// may target for it to name something. A refusal suggests the first.
    target_keys: Vec<(&'static str, &'static [TargetKind])>,
}

impl<'l> Mentions<'l> {
    /// What `conds` and `costs` name, and the owners `exprs` read, as `a number`: `subject:`,
    /// `id:` and `of:` on a condition, then a cost's `items:`.
    pub(crate) fn of_lines(
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
                Cond::Meter { of, .. } => mentions.add_owner("of:", of),
                Cond::Flag { of, .. } => mentions.add_flag_owner("of:", of),
                _ => {}
            });
        }
        for (id, _) in costs.into_iter().flat_map(|cost| &cost.items) {
            mentions.add_item_id("a cost's items:", id);
        }
        for read in exprs.into_iter().flat_map(Expr::reads) {
            if let Expr::Meter { of, .. } = read {
                mentions.add_owner("a number", of);
            }
        }
        mentions
    }

    fn add_owner(&mut self, key: &'static str, of: &grammar::Object) {
        match of {
            grammar::Object::Actor(actor) => self.add_actor(key, actor),
            of => self.owners.push(of.clone()),
        }
    }

    /// As `add_owner`, but `target` may also be an exit: a flag on its path.
    fn add_flag_owner(&mut self, key: &'static str, of: &grammar::Object) {
        match of {
            grammar::Object::Actor(grammar::Actor::Target) => {
                self.add_target(key, &[TargetKind::Actor, TargetKind::Path]);
            }
            of => self.add_owner(key, of),
        }
    }

    fn add_actor(&mut self, key: &'static str, actor: &grammar::Actor) {
        match actor {
            grammar::Actor::Target => self.add_target(key, &[TargetKind::Actor]),
            actor => self.owners.push(grammar::Object::Actor(actor.clone())),
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

    /// The first of the target keys where a move that targets `targets` does not allow
    /// `target`, as the key and what it needs the move to target. A habit, a change rule and an
    /// exit target nothing.
    pub(crate) fn find_misplaced_target(
        self,
        targets: TargetKind,
    ) -> Option<(&'static str, TargetKind)> {
        let misplaced =
            (self.target_keys.into_iter()).find(|(_, allowed)| !allowed.contains(&targets));
        misplaced.map(|(key, allowed)| (key, allowed[0]))
    }
}

/// What the start or the finish of an action costs, and whether it uses the actor's act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Phase {
    pub cost: Option<Cost>,
    pub counts_as_act: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Owner {
    Verb,
    Place(String),
    Item(String),
    Cast(String),
    Story(String),
}

impl Owner {
    pub(crate) fn rank(&self) -> u8 {
        match self {
            Owner::Verb => 0,
            Owner::Place(_) => 1,
            Owner::Item(_) => 2,
            Owner::Cast(_) => 3,
            Owner::Story(_) => 4,
        }
    }

    pub(crate) fn rule_id(&self, local: &str) -> String {
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
pub(crate) enum When {
    Verb { verb_id: String, target: TargetKind },
    Habit,
    Change(ChangeWatch),
}

impl When {
    /// What the rule's move targets. A habit and a change rule target nothing.
    pub(crate) fn target_kind(&self) -> TargetKind {
        match self {
            When::Verb { target, .. } => *target,
            When::Habit | When::Change(_) => TargetKind::None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    None,
    Actor,
    Place,
    Path,
    Item,
}

impl TargetKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TargetKind::None => "none",
            TargetKind::Actor => "actor",
            TargetKind::Place => "place",
            TargetKind::Path => "path",
            TargetKind::Item => "item",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ChangeWatch {
    Arrive {
        place: String,
        subject: grammar::Subject,
    },
    Leave {
        place: String,
        subject: grammar::Subject,
    },
    Meter {
        key: String,
        op: Op,
        bound: i64,
    },
    Flag {
        key: String,
        becomes: bool,
    },
    Item {
        id: String,
        gained: bool,
    },
}

/// A `check:`, a rule's or a nested one: `total` against `success_at`, with an optional lower
/// `partial_at` band.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Check {
    pub total: Expr,
    pub success_at: Expr,
    pub partial_at: Option<Expr>,
}

impl Check {
    /// The total, the success bar, then the partial band.
    pub(crate) fn exprs(&self) -> impl Iterator<Item = &Expr> {
        [&self.total, &self.success_at]
            .into_iter()
            .chain(&self.partial_at)
    }

    /// Evaluate `total`, then `success_at`, then `partial_at` on `dice`, each writing its own
    /// dice line, and classify the total.
    pub(crate) fn roll(
        &self,
        context: &Context,
        dice: &mut dyn expr::Dice,
    ) -> Result<Roll, Reason> {
        let total = expr::eval_with_dice(context, &self.total, dice)?;
        let success_at = expr::eval_with_dice(context, &self.success_at, dice)?;
        let partial_at = (self.partial_at.as_ref())
            .map(|expr| expr::eval_with_dice(context, expr, dice))
            .transpose()?;
        Ok(Roll {
            total,
            success_at,
            outcome: classify(total, success_at, partial_at),
        })
    }
}

/// Where a parsed meter read sits in the pack file. Line and column are 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LineColumn {
    pub line: u32,
    pub column: u32,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cost {
    pub ticks: Expr,
    pub meters: Vec<(String, Expr)>,
    pub items: Vec<(String, Expr)>,
}

impl Cost {
    /// The ticks, the meter amounts, then the item counts.
    pub(crate) fn exprs(&self) -> impl Iterator<Item = &Expr> {
        let amounts = self.meters.iter().chain(&self.items).map(|(_, expr)| expr);
        std::iter::once(&self.ticks).chain(amounts)
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    HasItem {
        subject: grammar::Actor,
        id: String,
        count: Expr,
    },
    Meter {
        of: grammar::Object,
        key: String,
        op: Op,
        value: Expr,
    },
    Flag {
        of: grammar::Object,
        key: String,
        is: bool,
    },
    At {
        subject: grammar::Actor,
        place: String,
    },
    /// A tick of cycle `id` in `from..to`, wrapping past the cycle's end when `from > to`.
    Cycle {
        id: String,
        from: Expr,
        to: Expr,
    },
    Players {
        op: Op,
        value: Expr,
    },
    NoteExists(String),
    Wielding {
        subject: grammar::Actor,
        id: String,
    },
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
}

impl Cond {
    /// Calls `visit` on every leaf condition, walking into `all`, `any` and `not`.
    pub(crate) fn visit<'a>(&'a self, mut visit: impl FnMut(&'a Cond)) {
        fn walk<'a>(cond: &'a Cond, visit: &mut impl FnMut(&'a Cond)) {
            match cond {
                Cond::All(inner) | Cond::Any(inner) => {
                    for cond in inner {
                        walk(cond, visit);
                    }
                }
                Cond::Not(inner) => walk(inner, visit),
                leaf => visit(leaf),
            }
        }
        walk(self, &mut visit);
    }

    /// Every number a leaf holds: an item count, a meter or player value, a cycle's bounds,
    /// walking into `all`, `any` and `not`.
    pub(crate) fn exprs(&self) -> Vec<&Expr> {
        let mut exprs = Vec::new();
        self.visit(|cond| match cond {
            Cond::HasItem { count: value, .. }
            | Cond::Meter { value, .. }
            | Cond::Players { value, .. } => exprs.push(value),
            Cond::Cycle { from, to, .. } => exprs.extend([from, to]),
            _ => {}
        });
        exprs
    }
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
    pub(crate) fn parse(word: &str) -> Option<Self> {
        match word {
            "<" => Some(Op::Lt),
            "<=" => Some(Op::Le),
            "==" => Some(Op::Eq),
            ">=" => Some(Op::Ge),
            ">" => Some(Op::Gt),
            _ => None,
        }
    }

    pub(crate) fn holds(self, left: i64, right: i64) -> bool {
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
pub(crate) enum EffectTemplate {
    Meter {
        of: grammar::Object,
        key: String,
        add: Option<Expr>,
        set: Option<Expr>,
    },
    Flag {
        of: grammar::Object,
        key: String,
        value: bool,
    },
    /// One application of modifier `id` to an actor, with magnitude `add`, lasting `ticks`.
    Modifier {
        to: grammar::Actor,
        id: String,
        add: Expr,
        ticks: Expr,
    },
    Location {
        to: LocationTo,
        teleport: bool,
    },
    Note {
        id: String,
        to_ids: Vec<ActorId>,
        place_id: Option<PlaceId>,
        text: String,
    },
    Narrate {
        to: NarrateTo,
        text: String,
    },
    Memory {
        to: grammar::Actor,
        text: Option<String>,
    },
    Item {
        of: grammar::Object,
        id: String,
        add: Expr,
        max: Option<Expr>,
    },
    Wield {
        subject: grammar::Actor,
        id: String,
    },
    Unwield {
        subject: grammar::Actor,
    },
    /// A save with its own outcome: `check` rolls on the rule's dice and is told to `by`, and
    /// its outcome picks the one list whose effects run. `self` is still the rule's actor.
    Check {
        by: grammar::Actor,
        check: Check,
        success: Vec<EffectTemplate>,
        partial: Vec<EffectTemplate>,
        miss: Vec<EffectTemplate>,
    },
}

impl EffectTemplate {
    /// Calls `visit` on this effect, then on every effect in a nested check's lists, walking
    /// into the nested checks they hold.
    pub(crate) fn visit<'a>(&'a self, mut visit: impl FnMut(&'a EffectTemplate)) {
        fn walk<'a>(effect: &'a EffectTemplate, visit: &mut impl FnMut(&'a EffectTemplate)) {
            visit(effect);
            if let EffectTemplate::Check {
                success,
                partial,
                miss,
                ..
            } = effect
            {
                for inner in success.iter().chain(partial).chain(miss) {
                    walk(inner, visit);
                }
            }
        }
        walk(self, &mut visit);
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LocationTo {
    Target,
    Place(String),
    Via(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NarrateTo {
    Here,
    Self_,
    Actors(Vec<ActorId>),
    /// No `to`: every seated player.
    Present,
}

#[derive(Clone, Debug)]
pub struct Table {
    rules: Vec<Rule>,
}

impl Table {
    pub(crate) fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter()
    }

    pub(crate) fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    pub(crate) fn habits_of(&self, actor: &ActorId) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| {
                matches!(&rule.owner, Owner::Cast(id) if *actor == **id)
                    && matches!(rule.when, When::Habit)
            })
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
