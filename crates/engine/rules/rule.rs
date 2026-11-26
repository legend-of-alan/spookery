//! One rule shape. The id is namespaced by the file that owns it.

use crate::ecs::id::{ActorId, PlaceId};
use crate::rules::expr::Expr;
use crate::rules::{cond, grammar, mentions, roll};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    pub id: String,
    pub local_id: String,
    pub scope: Scope,
    pub kind: RuleKind,
    pub if_: Vec<Cond>,
    /// `let:`: each id and the number it stores, in the order written. They roll first, and
    /// the rule's check, effects and later ids read them by id.
    pub lets: Vec<(String, Expr)>,
    pub check: Option<roll::Check>,
    /// Paid when the action starts. Its `cost` is the rule's `cost`, whose ticks are how long
    /// the action takes.
    pub start: ActionCost,
    /// Paid when the action finishes.
    pub finish: ActionCost,
    /// Ticks the actor stays busy after the finish.
    pub cooldown_ticks: u64,
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
        exprs.extend(self.check.iter().flat_map(roll::Check::exprs));
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

    /// The entities, item ids and `target`s the rule's lines name: its conditions', costs' and
    /// numbers', then on its effects `of:`, `subject:`, `to:`, a nested check's `by:`, `id:`, and
    /// `location to:` or `location via:` when it says `target`.
    pub(crate) fn mentions(&self) -> mentions::Mentions<'_> {
        let mut mentions = mentions::Mentions::of_lines(&self.if_, self.costs(), self.exprs());
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

/// What the start or the finish of an action costs, and whether it uses the actor's act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ActionCost {
    pub cost: Option<Cost>,
    pub counts_as_act: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Scope {
    Verb,
    Place(String),
    Item(String),
    Cast(String),
    Story(String),
}

impl Scope {
    pub(crate) fn rank(&self) -> u8 {
        match self {
            Scope::Verb => 0,
            Scope::Place(_) => 1,
            Scope::Item(_) => 2,
            Scope::Cast(_) => 3,
            Scope::Story(_) => 4,
        }
    }

    pub(crate) fn rule_id(&self, local: &str) -> String {
        match self {
            Scope::Verb => format!("verb:{local}"),
            Scope::Place(id) => format!("place:{id}/{local}"),
            Scope::Item(id) => format!("item:{id}/{local}"),
            Scope::Cast(id) => format!("cast:{id}/{local}"),
            Scope::Story(id) => format!("story:{id}/{local}"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum RuleKind {
    // repeats module: the concept map's word, beside TargetKind
    Verb { verb_id: String, target: TargetKind },
    Habit,
    Change(ChangeWatch),
}

impl RuleKind {
    /// The kind of thing the rule's verb targets. A habit and a change rule target nothing.
    pub(crate) fn target_kind(&self) -> TargetKind {
        match self {
            RuleKind::Verb { target, .. } => *target,
            RuleKind::Habit | RuleKind::Change(_) => TargetKind::None,
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
        place_id: String,
        subject: grammar::Subject,
    },
    Leave {
        place_id: String,
        subject: grammar::Subject,
    },
    Meter {
        id: String,
        op: cond::Op,
        bound: i64,
    },
    Flag {
        id: String,
        becomes: bool,
    },
    Item {
        id: String,
        gained: bool,
    },
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
        id: String,
        op: cond::Op,
        value: Expr,
    },
    Flag {
        of: grammar::Object,
        id: String,
        is: bool,
    },
    At {
        subject: grammar::Actor,
        place_id: String,
    },
    /// A tick of cycle `id` in `from..to`, wrapping past the cycle's end when `from > to`.
    Cycle {
        id: String,
        from: Expr,
        to: Expr,
    },
    Players {
        op: cond::Op,
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum EffectTemplate {
    Meter {
        of: grammar::Object,
        id: String,
        add: Option<Expr>,
        set: Option<Expr>,
    },
    Flag {
        of: grammar::Object,
        id: String,
        value: bool,
    },
    /// One application of modifier `id` to an actor, with magnitude `add`, lasting `ticks`.
    Modifier {
        to: grammar::Actor,
        id: String,
        add: Expr,
        ticks: Expr,
    },
    Move {
        to: MoveTo,
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
        check: roll::Check,
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
pub(crate) enum MoveTo {
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
pub struct Rules {
    rules: Vec<Rule>,
}

impl Rules {
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
                matches!(&rule.scope, Scope::Cast(id) if *actor == **id)
                    && matches!(rule.kind, RuleKind::Habit)
            })
            .collect()
    }

    pub fn ids_in_order(&self) -> Vec<&str> {
        let mut rules: Vec<&Rule> = self.rules.iter().collect();
        rules.sort_by(|left, right| {
            left.scope
                .rank()
                .cmp(&right.scope.rank())
                .then(left.file.cmp(&right.file))
                .then(left.index.cmp(&right.index))
        });
        rules.into_iter().map(|rule| rule.id.as_str()).collect()
    }
}
