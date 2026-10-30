//! One rule shape. The id is namespaced by the file that owns it.

use crate::rules::expr::Expr;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rule {
    pub id: String,
    pub local_id: String,
    pub owner: Owner,
    pub when: When,
    pub if_: Vec<Cond>,
    pub roll: Option<RollSpec>,
    pub cost: Option<Cost>,
    pub do_: Vec<EffectTemplate>,
    pub success: Vec<EffectTemplate>,
    pub partial: Vec<EffectTemplate>,
    pub failure: Vec<EffectTemplate>,
    pub file: String,
    pub index: usize,
}

impl Rule {
    pub fn watched_place<'a>(&'a self, place: &'a str) -> &'a str {
        if place == "here" {
            if let Owner::Place(id) = &self.owner {
                return id;
            }
        }
        place
    }

    pub fn top_templates(&self) -> impl Iterator<Item = &EffectTemplate> {
        self.do_
            .iter()
            .chain(&self.success)
            .chain(&self.partial)
            .chain(&self.failure)
    }

    pub fn visit_templates<'a>(&'a self, mut visit: impl FnMut(&'a EffectTemplate)) {
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
        walk(&self.failure, &mut visit);
    }
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
    pub fn rank(&self) -> u8 {
        match self {
            Owner::Verb => 0,
            Owner::Place(_) => 1,
            Owner::Item(_) => 2,
            Owner::Cast(_) => 3,
            Owner::Story(_) => 4,
        }
    }

    pub fn rule_id(&self, local: &str) -> String {
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
    Verb { verb: String, target: TargetKind },
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
    pub fn as_str(self) -> &'static str {
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
pub enum WatchWho {
    Any,
    Pc,
    Npc,
    Actor(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ChangeWatch {
    Arrive { place: String, who: WatchWho },
    Leave { place: String, who: WatchWho },
    Meter { key: String, op: Op, bound: i64 },
    Flag { key: String, becomes: bool },
    Clock { id: String },
    Item { id: String, gained: bool },
    Knows { fact: String },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RollSpec {
    pub check: String,
    pub target: Expr,
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
    pub minutes: u64,
    pub meters: Vec<(String, Expr)>,
    pub items: Vec<(String, Expr)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Cond {
    HasItem {
        who: Who,
        id: String,
        count: u32,
    },
    Meter {
        who: Who,
        key: String,
        op: Op,
        value: Expr,
    },
    Flag {
        who: Who,
        key: String,
        is: bool,
    },
    At {
        who: Who,
        place: String,
    },
    Knows {
        who: Who,
        fact: String,
    },
    Clock {
        id: String,
        op: Op,
        value: Expr,
    },
    Hour {
        from: i64,
        to: i64,
    },
    Players {
        op: Op,
        value: i64,
    },
    NoteExists(String),
    Wielding {
        who: Who,
        id: String,
    },
    All(Vec<Cond>),
    Any(Vec<Cond>),
    Not(Box<Cond>),
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
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "<" => Some(Op::Lt),
            "<=" => Some(Op::Le),
            "==" => Some(Op::Eq),
            ">=" => Some(Op::Ge),
            ">" => Some(Op::Gt),
            _ => None,
        }
    }

    pub fn holds(self, left: i64, right: i64) -> bool {
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
pub enum Who {
    Self_,
    Target,
    Here,
    World,
    Place(String),
    Actor(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EffectTemplate {
    Meter {
        who: Who,
        key: String,
        add: Option<Expr>,
        set: Option<Expr>,
        resist: Option<ResistSpec>,
    },
    Flag {
        who: Who,
        key: String,
        value: bool,
        resist: Option<ResistSpec>,
    },
    Location {
        to: LocationTo,
        teleport: bool,
    },
    Note {
        id: String,
        to: Vec<String>,
        place: Option<String>,
        fact_ids: Vec<String>,
        text: String,
    },
    Narrate {
        to: NarrateTo,
        text: String,
        fact_ids: Vec<String>,
        resist: Option<ResistSpec>,
    },
    Clock {
        id: String,
        add: Option<i64>,
        start: bool,
    },
    Memory {
        who: Who,
        fact_ids: Vec<String>,
        text: Option<String>,
    },
    Item {
        who: Who,
        id: String,
        add: i64,
        max: Option<i64>,
        resist: Option<ResistSpec>,
    },
    Reveal {
        who: Who,
        fact: String,
    },
    Wield {
        who: Who,
        id: String,
    },
    Unwield {
        who: Who,
    },
}

impl EffectTemplate {
    pub fn resist(&self) -> Option<&ResistSpec> {
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
    Actors(Vec<String>),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ResistSpec {
    Same,
    Roll {
        who: Who,
        check: String,
        target: Expr,
        resisted: Vec<EffectTemplate>,
    },
}

#[derive(Clone, Debug)]
pub struct RuleTable {
    rules: Vec<Rule>,
}

impl RuleTable {
    pub fn new(rules: Vec<Rule>) -> Self {
        Self { rules }
    }

    pub fn iter(&self) -> impl Iterator<Item = &Rule> {
        self.rules.iter()
    }

    pub fn get(&self, id: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| rule.id == id)
    }

    pub fn verb(&self, name: &str) -> Option<&Rule> {
        self.rules.iter().find(|rule| {
            matches!(&rule.owner, Owner::Verb)
                && matches!(&rule.when, When::Verb { verb, .. } if verb == name)
        })
    }

    pub fn named(&self, local: &str) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| rule.local_id == local || rule.id == local)
            .collect()
    }

    pub(crate) fn habits_of(&self, actor: &str) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| {
                matches!(&rule.owner, Owner::Cast(id) if id == actor)
                    && matches!(rule.when, When::Habit)
            })
            .collect()
    }

    /// Verbs the player can type, in file order.
    pub(crate) fn player_verbs(&self) -> Vec<&Rule> {
        self.rules
            .iter()
            .filter(|rule| matches!(rule.when, When::Verb { .. }))
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
