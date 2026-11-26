//! Maps keyed by a parent's id and an entry's id, written as a nested map by parent: the
//! pair-keyed map `{(mira, vigor): meter}` is `mira: { vigor: meter }` in the save, and a parent
//! with no entries writes nothing.

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The key `id` has under `parent`. It clones both: std's maps look a key up by a borrowed
/// form only through `Borrow`, which a pair of owned strings cannot give to `(&Id, &str)`.
fn key_of<Id: Clone>(parent: &Id, id: &str) -> (Id, String) {
    (parent.clone(), id.to_string())
}

/// The entry `id` that `parent` holds in a map keyed by `(parent, id)`.
pub(crate) fn get<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    parent: &Id,
    id: &str,
) -> Option<&'a V> {
    map.get(&key_of(parent, id))
}

/// The entries `parent` holds in a map keyed by `(parent, id)`, in id order.
pub(crate) fn entries_of<'a, Id: Ord + Clone, V>(
    map: &'a BTreeMap<(Id, String), V>,
    parent: &'a Id,
) -> impl Iterator<Item = (&'a str, &'a V)> {
    map.range(key_of(parent, "")..)
        .take_while(move |((held_by, _), _)| held_by == parent)
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
    for ((parent, id), value) in map {
        grouped.entry(parent).or_default().insert(id, value);
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
    let pairs = grouped.into_iter().flat_map(|(parent, entries)| {
        entries
            .into_iter()
            .map(move |(id, value)| ((parent.clone(), id), value))
    });
    Ok(pairs.collect::<BTreeMap<_, _>>().into())
}

/// The ids set on each parent in a map of `(parent, id)` to `()`, such as the flags set on each
/// actor, written as each parent's list of ids: `mira: [calm, wary]`.
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
        for (parent, id) in set.keys() {
            grouped.entry(parent).or_default().push(id);
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
            .flat_map(|(parent, ids)| ids.into_iter().map(move |id| ((parent.clone(), id), ())));
        Ok(pairs.collect::<BTreeMap<_, _>>().into())
    }
}

/// A pair-keyed map whose one parent the save leaves out, such as the world's meters:
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

    /// The ids set on that one parent, written as a list: `[calm, wary]`.
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
