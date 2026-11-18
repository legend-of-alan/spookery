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
        exprs.extend(self.costs().flat_map(Cost::exprs));
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

    /// Every slot the rule's lines fill, with its key as written: its conditions' and costs'
    /// slots, the owners its numbers read, and on its effects `of:`, `subject:`, `to:`, a
    /// resist's `subject:`, `id:`, and `location to:` or `location via:` when it says `target`.
    pub(crate) fn slots(&self) -> Vec<(&'static str, Slot<'_>)> {
        let mut slots = collect_slots(&self.if_, self.costs(), self.exprs());
        self.visit_templates(|effect| {
            if let Some(ResistSpec::Roll { subject, .. }) = effect.resist() {
                slots.push(("a resist's subject:", Slot::from_actor(subject)));
            }
            match effect {
                EffectTemplate::Meter { of, .. }
                | EffectTemplate::Flag { of, .. }
                | EffectTemplate::Item { of, .. } => slots.push(("of:", Slot::from_owner(of))),
                EffectTemplate::Modifier { to, .. }
                | EffectTemplate::Memory { to, .. }
                | EffectTemplate::Reveal { to, .. } => slots.push(("to:", Slot::from_actor(to))),
                EffectTemplate::Wield { subject, .. } | EffectTemplate::Unwield { subject } => {
                    slots.push(("subject:", Slot::from_actor(subject)));
                }
                EffectTemplate::Location { to, .. } => match to {
                    LocationTo::Target => {
                        slots.push(("location to:", Slot::Target(TargetKind::Place)));
                    }
                    LocationTo::Via(via) if via == "target" => {
                        slots.push(("location via:", Slot::Target(TargetKind::Exit)));
                    }
                    LocationTo::Via(_) | LocationTo::Place(_) => {}
                },
                EffectTemplate::Note { .. } | EffectTemplate::Narrate { .. } => {}
            }
            if let EffectTemplate::Item { id, .. } | EffectTemplate::Wield { id, .. } = effect {
                slots.push(("id:", Slot::from_item_id(id)));
            }
        });
        slots
    }
}

/// What one slot of a rule or exit line names, borrowed from that line (`'l`). The word
/// `target` stands for whatever the move targets, so it is kept apart from the owners and items
/// it could name.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Slot<'l> {
    /// An owner other than `target`.
    Owner(grammar::Object),
    /// An item id other than `target`.
    Item(&'l str),
    /// The word `target`, with what the move must target for it to name something.
    Target(TargetKind),
}

impl<'l> Slot<'l> {
    fn from_owner(of: &grammar::Object) -> Self {
        match of {
            grammar::Object::Actor(actor) => Slot::from_actor(actor),
            of => Slot::Owner(of.clone()),
        }
    }

    fn from_actor(actor: &grammar::Actor) -> Self {
        match actor {
            grammar::Actor::Target => Slot::Target(TargetKind::Actor),
            actor => Slot::Owner(grammar::Object::Actor(actor.clone())),
        }
    }

    fn from_item_id(id: &'l str) -> Self {
        match id {
            "target" => Slot::Target(TargetKind::Item),
            id => Slot::Item(id),
        }
    }
}

/// Every slot `conds` and `costs` fill, and every owner `exprs` read, as `a number`, with each
/// slot's key as written: `subject:`, `id:` and `of:` on a condition, and a cost's `items:`.
pub(crate) fn collect_slots<'l>(
    conds: &'l [Cond],
    costs: impl IntoIterator<Item = &'l Cost>,
    exprs: Vec<&Expr>,
) -> Vec<(&'static str, Slot<'l>)> {
    let mut slots = Vec::new();
    for cond in conds {
        cond.visit(|cond| match cond {
            Cond::HasItem { subject, id, .. } | Cond::Wielding { subject, id } => {
                slots.push(("subject:", Slot::from_actor(subject)));
                slots.push(("id:", Slot::from_item_id(id)));
            }
            Cond::At { subject, .. } | Cond::Knows { subject, .. } => {
                slots.push(("subject:", Slot::from_actor(subject)));
            }
            Cond::Meter { of, .. } | Cond::Flag { of, .. } => {
                slots.push(("of:", Slot::from_owner(of)));
            }
            _ => {}
        });
    }
    let items = costs.into_iter().flat_map(|cost| &cost.items);
    slots.extend(items.map(|(id, _)| ("a cost's items:", Slot::from_item_id(id))));
    let reads = exprs.into_iter().flat_map(Expr::meter_reads);
    slots.extend(reads.map(|(of, _, _)| ("a number", Slot::from_owner(of))));
    slots
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
