//! One rule shape. The id is namespaced by the file it lives in.

use crate::ecs::id::{ActorId, PlaceId};
use crate::rules::cond::{self, Cond};
use crate::rules::cost::Cost;
use crate::rules::expr::Expr;
use crate::rules::{grammar, roll};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Rule {
    pub id: String,
    pub local_id: String,
    pub scope: Scope,
    pub kind: Kind,
    pub if_: Vec<Cond>,
    /// `let:`: each id and the number it stores, in the order written. They roll first, and
    /// the rule's check, effects and later ids read them by id.
    pub lets: Vec<(String, Expr)>,
    pub check: Option<roll::Check>,
    /// The rule's `cost`, paid when the action starts. Its ticks are how long the action takes.
    pub start_cost: Option<Cost>,
    pub finish_cost: Option<Cost>,
    pub start_counts_as_act: bool,
    pub finish_counts_as_act: bool,
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
    pub(crate) fn costs(&self) -> impl Iterator<Item = &Cost> {
        self.start_cost.iter().chain(&self.finish_cost)
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
pub(crate) enum Kind {
    Verb { verb_id: String, target: TargetKind },
    Habit,
    Change(ChangeWatch),
}

impl Kind {
    /// The kind of thing the rule's verb targets. A habit and a change rule target nothing.
    pub(crate) fn target_kind(&self) -> TargetKind {
        match self {
            Kind::Verb { target, .. } => *target,
            Kind::Habit | Kind::Change(_) => TargetKind::None,
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
                    && matches!(rule.kind, Kind::Habit)
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
