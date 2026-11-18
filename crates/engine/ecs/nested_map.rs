//! Maps keyed by an owner's id and an entry's id, written as a nested map by owner: the
//! pair-keyed map `{(mira, vigor): meter}` is `mira: { vigor: meter }` in the save, and an owner
//! with no entries writes nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The key `id` has under `owner`. It clones both: std's maps look a key up by a borrowed
/// form only through `Borrow`, which a pair of owned strings cannot give to `(&Id, &str)`.
fn key_of<Id: Clone>(owner: &Id, id: &str) -> (Id, String) {
    (owner.clone(), id.to_string())
}

/// The entry `id` that `owner` holds in a map keyed by `(owner, id)`.
pub(crate) fn get<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &Id,
    id: &str,
) -> Option<&'a V> {
    map.get(&key_of(owner, id))
}

/// The entries `owner` holds in a map keyed by `(owner, id)`, in id order.
pub(crate) fn entries_of<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    owner: &'a Id,
) -> impl Iterator<Item = (&'a str, &'a V)> {
    map.range(key_of(owner, "")..)
        .take_while(move |((held_by, _), _)| held_by == owner)
        .map(|((_, id), value)| (id.as_str(), value))
}

pub(crate) fn serialize<S, Id, V>(
    map: &BTreeMap<(Id, String), V>,
    serializer: S,
) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    Id: Serialize + Ord,
    V: Serialize,
{
    let mut grouped: BTreeMap<&Id, BTreeMap<&str, &V>> = BTreeMap::new();
    for ((owner, id), value) in map {
        grouped.entry(owner).or_default().insert(id, value);
    }
    grouped.serialize(serializer)
}

/// The map a nested map in the file reads as, or the store that holds it.
pub(crate) fn deserialize<'de, D, Id, V, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    Id: Deserialize<'de> + Ord + Clone,
    V: Deserialize<'de>,
    T: From<BTreeMap<(Id, String), V>>,
{
    let grouped = BTreeMap::<Id, BTreeMap<String, V>>::deserialize(deserializer)?;
    let pairs = grouped.into_iter().flat_map(|(owner, entries)| {
        entries
            .into_iter()
            .map(move |(id, value)| ((owner.clone(), id), value))
    });
    Ok(pairs.collect::<BTreeMap<_, _>>().into())
}

/// The ids set on each owner in a map of `(owner, id)` to `()`, such as the flags set on each
/// actor, written as each owner's list of ids: `mira: [calm, wary]`.
pub(crate) mod lists {
    use super::*;

    pub(crate) fn serialize<S, Id>(
        set: &BTreeMap<(Id, String), ()>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        Id: Serialize + Ord,
    {
        let mut grouped: BTreeMap<&Id, Vec<&str>> = BTreeMap::new();
        for (owner, id) in set.keys() {
            grouped.entry(owner).or_default().push(id);
        }
        grouped.serialize(serializer)
    }

    pub(crate) fn deserialize<'de, D, Id, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        Id: Deserialize<'de> + Ord + Clone,
        T: From<BTreeMap<(Id, String), ()>>,
    {
        let grouped = BTreeMap::<Id, Vec<String>>::deserialize(deserializer)?;
        let pairs = grouped
            .into_iter()
            .flat_map(|(owner, ids)| ids.into_iter().map(move |id| ((owner.clone(), id), ())));
        Ok(pairs.collect::<BTreeMap<_, _>>().into())
    }
}

/// A pair-keyed map whose one owner the save leaves out, such as the world's meters:
/// `{(world, tide): meter}` is `tide: meter`.
pub(crate) mod flat {
    use super::*;

    pub(crate) fn serialize<S, Id, V>(
        map: &BTreeMap<(Id, String), V>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
        V: Serialize,
    {
        serializer.collect_map(map.iter().map(|((_, id), value)| (id, value)))
    }

    pub(crate) fn deserialize<'de, D, Id, V, T>(deserializer: D) -> Result<T, D::Error>
    where
        D: Deserializer<'de>,
        Id: Default + Ord,
        V: Deserialize<'de>,
        T: From<BTreeMap<(Id, String), V>>,
    {
        let entries = BTreeMap::<String, V>::deserialize(deserializer)?;
        let pairs = entries
            .into_iter()
            .map(|(id, value)| ((Id::default(), id), value));
        Ok(pairs.collect::<BTreeMap<_, _>>().into())
    }

    /// The ids set on that one owner, written as a list: `[calm, wary]`.
    pub(crate) mod lists {
        use super::*;

        pub(crate) fn serialize<S, Id>(
            set: &BTreeMap<(Id, String), ()>,
            serializer: S,
        ) -> Result<S::Ok, S::Error>
        where
            S: Serializer,
        {
            serializer.collect_seq(set.keys().map(|(_, id)| id))
        }

        pub(crate) fn deserialize<'de, D, Id, T>(deserializer: D) -> Result<T, D::Error>
        where
            D: Deserializer<'de>,
            Id: Default + Ord,
            T: From<BTreeMap<(Id, String), ()>>,
        {
            let ids = Vec::<String>::deserialize(deserializer)?;
            let pairs = ids.into_iter().map(|id| ((Id::default(), id), ()));
            Ok(pairs.collect::<BTreeMap<_, _>>().into())
        }
    }
}
