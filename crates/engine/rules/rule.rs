//! One rule shape. The id is namespaced by the file that owns it.

use crate::ecs::id::{ActorId, PlaceId};
use crate::rules::expr::Expr;
use crate::rules::grammar;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub local_id: String,
    pub owner: Owner,
    pub when: When,
    pub if_: Vec<Cond>,
    pub roll: Option<RollSpec>,
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

    pub(crate) fn top_templates(&self) -> impl Iterator<Item = &EffectTemplate> {
        self.do_
            .iter()
            .chain(&self.success)
            .chain(&self.partial)
            .chain(&self.miss)
    }

    pub(crate) fn visit_templates<'a>(&'a self, mut visit: impl FnMut(&'a EffectTemplate)) {
        fn walk<'a>(effects: &'a [EffectTemplate], visit: &mut impl FnMut(&'a EffectTemplate)) {
            for effect in effects {
                visit(effect);
                if let Some(ResistSpec::Roll { resisted, .. }) = effect.resist() {
                    walk(resisted, visit);
                }
            }
        }
        walk(&self.do_, &mut visit);
        walk(&self.success, &mut visit);
        walk(&self.partial, &mut visit);
        walk(&self.miss, &mut visit);
    }

    /// Every number the rule holds: its roll's threshold and band, its costs, its conditions'
    /// numbers, and its effects' amounts, caps, ticks and resist thresholds.
    pub(crate) fn exprs(&self) -> Vec<&Expr> {
        let mut exprs = Vec::new();
        if let Some(roll) = &self.roll {
            exprs.push(&roll.threshold);
            exprs.extend(&roll.partial_at);
        }
        for cost in self.costs() {
            exprs.extend(cost.exprs());
        }
        exprs.extend(self.if_.iter().flat_map(Cond::exprs));
        self.visit_templates(|effect| {
            match effect {
                EffectTemplate::Modifier { add, ticks, .. } => exprs.extend([add, ticks]),
                EffectTemplate::Meter { add, set, .. } => exprs.extend(add.iter().chain(set)),
                EffectTemplate::Item { add, max, .. } => exprs.extend([add].into_iter().chain(max)),
                _ => {}
            }
            if let Some(ResistSpec::Roll { threshold, .. }) = effect.resist() {
                exprs.push(threshold);
            }
        });
        exprs
    }

    /// Every owner the rule names, with the slot it sits in: `of:`, `subject:`, `to:`, a
    /// resist's `subject:`, or a number that reads one of its meters.
    pub(crate) fn objects(&self) -> Vec<(&'static str, grammar::Object)> {
        let mut objects = collect_objects(&self.if_, self.exprs());
        self.visit_templates(|effect| {
            if let Some(ResistSpec::Roll { subject, .. }) = effect.resist() {
                let subject = grammar::Object::Actor(subject.clone());
                objects.push(("a resist's subject:", subject));
            }
            let (slot, object) = match effect {
                EffectTemplate::Meter { of, .. }
                | EffectTemplate::Flag { of, .. }
                | EffectTemplate::Item { of, .. } => ("of:", of.clone()),
                EffectTemplate::Modifier { to, .. }
                | EffectTemplate::Memory { to, .. }
                | EffectTemplate::Reveal { to, .. } => ("to:", grammar::Object::Actor(to.clone())),
                EffectTemplate::Wield { subject, .. } | EffectTemplate::Unwield { subject } => {
                    ("subject:", grammar::Object::Actor(subject.clone()))
                }
                EffectTemplate::Location { .. }
                | EffectTemplate::Note { .. }
                | EffectTemplate::Narrate { .. } => return,
            };
            objects.push((slot, object));
        });
        objects
    }

    /// Every item id the rule names, with its slot: `id:` on a condition, an item effect or a
    /// `wield`, and a cost's `items:`.
    pub(crate) fn item_ids(&self) -> Vec<(&'static str, &str)> {
        let mut ids = collect_item_ids(&self.if_, self.costs());
        self.visit_templates(|effect| {
            if let EffectTemplate::Item { id, .. } | EffectTemplate::Wield { id, .. } = effect {
                ids.push(("id:", id));
            }
        });
        ids
    }
}

/// Every owner `conds` name in a `subject:` or `of:`, with that slot, and every owner `exprs`
/// read, as `a number`.
pub(crate) fn collect_objects<'a>(
    conds: &[Cond],
    exprs: impl IntoIterator<Item = &'a Expr>,
) -> Vec<(&'static str, grammar::Object)> {
    let mut objects = Vec::new();
    for cond in conds {
        cond.visit(|cond| match cond {
            Cond::HasItem { subject, .. }
            | Cond::At { subject, .. }
            | Cond::Knows { subject, .. }
            | Cond::Wielding { subject, .. } => {
                objects.push(("subject:", grammar::Object::Actor(subject.clone())));
            }
            Cond::Meter { of, .. } | Cond::Flag { of, .. } => objects.push(("of:", of.clone())),
            _ => {}
        });
    }
    for expr in exprs {
        let reads = expr.meter_reads().into_iter();
        objects.extend(reads.map(|(of, _, _)| ("a number", of.clone())));
    }
    objects
}

/// Every item id `conds` and `costs` name, with its slot: `id:` on `has_item` and `wielding`,
/// and a cost's `items:`.
pub(crate) fn collect_item_ids<'a>(
    conds: &'a [Cond],
    costs: impl IntoIterator<Item = &'a Cost>,
) -> Vec<(&'static str, &'a str)> {
    let mut ids = Vec::new();
    for cond in conds {
        cond.visit(|cond| {
            if let Cond::HasItem { id, .. } | Cond::Wielding { id, .. } = cond {
                ids.push(("id:", id.as_str()));
            }
        });
    }
    for cost in costs {
        ids.extend(
            cost.items
                .iter()
                .map(|(id, _)| ("a cost's items:", id.as_str())),
        );
    }
    ids
}

/// What the start or the finish of an action costs, and whether it uses the actor's act.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Phase {
    pub cost: Option<Cost>,
    pub counts_as_act: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Owner {
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
pub enum When {
    Verb { verb_id: String, target: TargetKind },
    Habit,
    Change(ChangeWatch),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetKind {
    None,
    Actor,
    Place,
    Exit,
    Item,
}

impl TargetKind {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            TargetKind::None => "none",
            TargetKind::Actor => "actor",
            TargetKind::Place => "place",
            TargetKind::Exit => "exit",
            TargetKind::Item => "item",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeWatch {
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
    Knows {
        fact: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollSpec {
    pub check: String,
    pub threshold: Expr,
    pub partial_at: Option<Expr>,
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
    Knows {
        subject: grammar::Actor,
        fact: String,
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
pub enum EffectTemplate {
    Meter {
        of: grammar::Object,
        key: String,
        add: Option<Expr>,
        set: Option<Expr>,
        resist: Option<ResistSpec>,
    },
    Flag {
        of: grammar::Object,
        key: String,
        value: bool,
        resist: Option<ResistSpec>,
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
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: NarrateTo,
        text: String,
        fact_ids: Vec<String>,
        resist: Option<ResistSpec>,
    },
    Memory {
        to: grammar::Actor,
        fact_ids: Vec<String>,
        text: Option<String>,
    },
    Item {
        of: grammar::Object,
        id: String,
        add: Expr,
        max: Option<Expr>,
        resist: Option<ResistSpec>,
    },
    Reveal {
        to: grammar::Actor,
        fact: String,
    },
    Wield {
        subject: grammar::Actor,
        id: String,
    },
    Unwield {
        subject: grammar::Actor,
    },
}

impl EffectTemplate {
    pub(crate) fn resist(&self) -> Option<&ResistSpec> {
        match self {
            EffectTemplate::Meter { resist, .. }
            | EffectTemplate::Flag { resist, .. }
            | EffectTemplate::Narrate { resist, .. }
            | EffectTemplate::Item { resist, .. } => resist.as_ref(),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocationTo {
    Target,
    Place(String),
    Via(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum NarrateTo {
    Here,
    Self_,
    Actors(Vec<ActorId>),
    /// No `to`: every seated player.
    Present,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResistSpec {
    Same,
    Roll {
        subject: grammar::Actor,
        check: String,
        threshold: Expr,
        resisted: Vec<EffectTemplate>,
    },
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
