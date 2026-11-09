//! Maps keyed by an owner's id and a name, written as a nested map by owner: the pair-keyed map
//! `{(mira, grit): meter}` is `mira: { grit: meter }` in the save, and an owner with no entries
//! writes nothing.

use std::borrow::Borrow;
use std::cmp::Ordering;
use std::collections::{BTreeMap, BTreeSet};
use std::ops::Bound;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A key of a map keyed by `(owner, name)`, owned or borrowed, so the map is read by
/// `(&owner, &name)` without building an owned key.
pub trait Pair<Id> {
    fn owner(&self) -> &Id;
    fn name(&self) -> &str;
}

impl<Id> Pair<Id> for (Id, String) {
    fn owner(&self) -> &Id {
        &self.0
    }
    fn name(&self) -> &str {
        &self.1
    }
}

impl<Id> Pair<Id> for (&Id, &str) {
    fn owner(&self) -> &Id {
        self.0
    }
    fn name(&self) -> &str {
        self.1
    }
}

impl<'a, Id: 'a> Borrow<dyn Pair<Id> + 'a> for (Id, String) {
    fn borrow(&self) -> &(dyn Pair<Id> + 'a) {
        self
    }
}

/// The order of `(Id, String)` itself, so a borrowed key finds what an owned one would.
impl<Id: Ord> Ord for dyn Pair<Id> + '_ {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.owner(), self.name()).cmp(&(other.owner(), other.name()))
    }
}

impl<Id: Ord> PartialOrd for dyn Pair<Id> + '_ {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl<Id: Ord> PartialEq for dyn Pair<Id> + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other).is_eq()
    }
}

impl<Id: Ord> Eq for dyn Pair<Id> + '_ {}

/// The entry `name` that `owner` holds in a map keyed by `(owner, name)`.
pub fn get<'a, Id: Ord, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &Id,
    name: &str,
) -> Option<&'a V> {
    map.get(&(owner, name) as &dyn Pair<Id>)
}

/// The same entry, to write.
pub(super) fn get_mut<'a, Id: Ord, V>(
    map: &'a mut BTreeMap<(Id, String), V>,
    owner: &Id,
    name: &str,
) -> Option<&'a mut V> {
    map.get_mut(&(owner, name) as &dyn Pair<Id>)
}

/// Whether `owner` holds `name` in a set of `(owner, name)`.
pub fn contains<Id: Ord>(set: &BTreeSet<(Id, String)>, owner: &Id, name: &str) -> bool {
    set.contains(&(owner, name) as &dyn Pair<Id>)
}

/// The entries `owner` holds in a map keyed by `(owner, name)`, in name order.
pub fn entries_of<'a, Id: Ord, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &'a Id,
) -> impl Iterator<Item = (&'a str, &'a V)> {
    let first = (owner, "");
    let from: Bound<&dyn Pair<Id>> = Bound::Included(&first);
    map.range::<dyn Pair<Id>, _>((from, Bound::Unbounded))
        .take_while(move |((held_by, _), _)| held_by == owner)
        .map(|((_, name), value)| (name.as_str(), value))
}

pub fn serialize<S, Id, V>(
    map: &BTreeMap<(Id, String), V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    Id: Serialize + Ord,
    V: Serialize,
{
    let mut grouped: BTreeMap<&Id, BTreeMap<&str, &V>> = BTreeMap::new();
    for ((owner, name), value) in map {
        grouped.entry(owner).or_default().insert(name, value);
    }
    grouped.serialize(serializer)
}

pub fn deserialize<'de, D, Id, V>(deserializer: D) -> Result<BTreeMap<(Id, String), V>, D::Error>
where
    D: Deserializer<'de>,
    Id: Deserialize<'de> + Ord + Clone,
    V: Deserialize<'de>,
{
    let grouped = BTreeMap::<Id, BTreeMap<String, V>>::deserialize(deserializer)?;
    let pairs = grouped.into_iter().flat_map(|(owner, named)| {
        named
            .into_iter()
            .map(move |(name, value)| ((owner.clone(), name), value))
    });
    Ok(pairs.collect())
}

/// A set of `(owner, name)`, such as the flags set on each actor, written as each owner's list
/// of names: `mira: [calm, wary]`.
pub mod lists {
    use super::*;

    pub fn serialize<S, Id>(set: &BTreeSet<(Id, String)>, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        Id: Serialize + Ord,
    {
        let mut grouped: BTreeMap<&Id, Vec<&str>> = BTreeMap::new();
        for (owner, name) in set {
            grouped.entry(owner).or_default().push(name);
        }
        grouped.serialize(serializer)
    }

    pub fn deserialize<'de, D, Id>(deserializer: D) -> Result<BTreeSet<(Id, String)>, D::Error>
    where
        D: Deserializer<'de>,
        Id: Deserialize<'de> + Ord + Clone,
    {
        let grouped = BTreeMap::<Id, Vec<String>>::deserialize(deserializer)?;
        let pairs = grouped
            .into_iter()
            .flat_map(|(owner, names)| names.into_iter().map(move |name| (owner.clone(), name)));
        Ok(pairs.collect())
    }
}
