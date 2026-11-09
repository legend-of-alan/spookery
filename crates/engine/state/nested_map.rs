//! Maps keyed by an owner's id and a name, written as a nested map by owner: the pair-keyed map
//! `{(mira, grit): meter}` is `mira: { grit: meter }` in the save, and an owner with no entries
//! writes nothing.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The entries `owner` holds in a map keyed by `(owner, name)`, in name order.
pub fn entries_of<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &Id,
) -> impl Iterator<Item = (&'a str, &'a V)> {
    let owner = owner.clone();
    map.range((owner.clone(), String::new())..)
        .take_while(move |((held_by, _), _)| *held_by == owner)
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
