//! Maps keyed by an owner's id and a name, written as a nested map by owner: the pair-keyed map
//! `{(mira, grit): meter}` is `mira: { grit: meter }` in the save, and an owner with no entries
//! writes nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The key `name` has under `owner`. It clones both: std's maps look a key up by a borrowed
/// form only through `Borrow`, which a pair of owned strings cannot give to `(&Id, &str)`.
fn key_of<Id: Clone>(owner: &Id, name: &str) -> (Id, String) {
    (owner.clone(), name.to_string())
}

/// The entry `name` that `owner` holds in a map keyed by `(owner, name)`.
pub fn get<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &Id,
    name: &str,
) -> Option<&'a V> {
    map.get(&key_of(owner, name))
}

/// The entries `owner` holds in a map keyed by `(owner, name)`, in name order.
pub fn entries_of<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &'a Id,
) -> impl Iterator<Item = (&'a str, &'a V)> {
    map.range(key_of(owner, "")..)
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

/// The map a nested map in the file reads as, or the store that holds it.
pub fn deserialize<'de, D, Id, V, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    Id: Deserialize<'de> + Ord + Clone,
    V: Deserialize<'de>,
    T: From<BTreeMap<(Id, String), V>>,
{
    let grouped = BTreeMap::<Id, BTreeMap<String, V>>::deserialize(deserializer)?;
    let pairs = grouped.into_iter().flat_map(|(owner, named)| {
        named
            .into_iter()
            .map(move |(name, value)| ((owner.clone(), name), value))
    });
    Ok(pairs.collect::<BTreeMap<_, _>>().into())
}

/// The names set on each owner in a map of `(owner, name)` to `()`, such as the flags set on each
/// actor, written as each owner's list of names: `mira: [calm, wary]`.
pub mod lists {
    use super::*;

    pub fn serialize<S, Id>(
        set: &BTreeMap<(Id, String), ()>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        Id: Serialize + Ord,
    {
        let mut grouped: BTreeMap<&Id, Vec<&str>> = BTreeMap::new();
        for (owner, name) in set.keys() {
            grouped.entry(owner).or_default().push(name);
        }
        grouped.serialize(serializer)
    }

    pub fn deserialize<'de, D, Id, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        Id: Deserialize<'de> + Ord + Clone,
        T: From<BTreeMap<(Id, String), ()>>,
    {
        let grouped = BTreeMap::<Id, Vec<String>>::deserialize(deserializer)?;
        let pairs = grouped.into_iter().flat_map(|(owner, names)| {
            names
                .into_iter()
                .map(move |name| ((owner.clone(), name), ()))
        });
        Ok(pairs.collect::<BTreeMap<_, _>>().into())
    }
}

/// A pair-keyed map whose one owner the save leaves out, such as the world's meters:
/// `{(world, tide): meter}` is `tide: meter`.
pub mod flat {
    use super::*;

    pub fn serialize<S, Id, V>(
        map: &BTreeMap<(Id, String), V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        serializer.collect_map(map.iter().map(|((_, name), value)| (name, value)))
    }

    pub fn deserialize<'de, D, Id, V, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        Id: Default + Ord,
        V: Deserialize<'de>,
        T: From<BTreeMap<(Id, String), V>>,
    {
        let named = BTreeMap::<String, V>::deserialize(deserializer)?;
        let pairs = named
            .into_iter()
            .map(|(name, value)| ((Id::default(), name), value));
        Ok(pairs.collect::<BTreeMap<_, _>>().into())
    }

    /// The names set on that one owner, written as a list: `[calm, wary]`.
    pub mod lists {
        use super::*;

        pub fn serialize<S, Id>(
            set: &BTreeMap<(Id, String), ()>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.collect_seq(set.keys().map(|(_, name)| name))
        }

        pub fn deserialize<'de, D, Id, T>(deserializer: D) -> Result<T, D::Error>
        where
            D: Deserializer<'de>,
            Id: Default + Ord,
            T: From<BTreeMap<(Id, String), ()>>,
        {
            let names = Vec::<String>::deserialize(deserializer)?;
            let pairs = names.into_iter().map(|name| ((Id::default(), name), ()));
            Ok(pairs.collect::<BTreeMap<_, _>>().into())
        }
    }
}
