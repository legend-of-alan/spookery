use serde_saphyr::SerializerOptions;

use crate::error::Code;
use crate::model::{sort_unique, Now, ProposalDoc};

#[allow(non_snake_case)]
pub fn NOW_EMIT_OPTIONS() -> SerializerOptions {
    serde_saphyr::ser_options! {
        empty_as_braces: true,
        indent_step: 2,
        compact_list_indent: false,
        anchor_generator: None,
        min_fold_chars: usize::MAX,
        folded_wrap_chars: usize::MAX,
        tagged_enums: false,
        prefer_block_scalars: false,
        quote_all: false,
        yaml_12: false,
    }
}

pub fn load_now(text: &str) -> Result<Now, Code> {
    let now: Now = serde_saphyr::from_str(text).map_err(|_| Code::Schema)?;
    if now.schema != "now/v0" {
        return Err(Code::Schema);
    }
    check_occupancy(&now)?;
    Ok(now)
}

pub fn emit_now(now: &Now) -> Result<String, Code> {
    let mut canonical = now.clone();
    canonicalize(&mut canonical);
    let text = serde_saphyr::to_string_with_options(&canonical, NOW_EMIT_OPTIONS())
        .map_err(|_| Code::Schema)?;
    Ok(one_trailing_lf(text))
}

pub fn emit_proposal(doc: &ProposalDoc) -> Result<String, Code> {
    if doc.schema != "proposal/v0" {
        return Err(Code::Schema);
    }
    let text = serde_saphyr::to_string_with_options(doc, NOW_EMIT_OPTIONS())
        .map_err(|_| Code::Schema)?;
    Ok(one_trailing_lf(text))
}

pub fn emit_rumors(now: &Now) -> Result<String, Code> {
    let mut rumors = now.rumors.clone();
    rumors.sort_by(|a, b| a.id.cmp(&b.id));
    for rumor in &mut rumors {
        let mut facts = rumor.fact_ids.clone();
        sort_unique(&mut facts);
        rumor.fact_ids = facts;
        let mut known = rumor.known_by.clone();
        sort_unique(&mut known);
        rumor.known_by = known;
    }
    let text = serde_saphyr::to_string_with_options(&rumors, NOW_EMIT_OPTIONS())
        .map_err(|_| Code::Schema)?;
    Ok(one_trailing_lf(text))
}

fn one_trailing_lf(mut text: String) -> String {
    while text.ends_with('\n') {
        text.pop();
    }
    text.push('\n');
    text
}

pub fn canonicalize(now: &mut Now) {
    sort_unique(&mut now.presence.session_ids);
    sort_unique(&mut now.presence.actors_present);
    sort_unique(&mut now.proposals);
    now.rumors.sort_by(|a, b| a.id.cmp(&b.id));
    for rumor in &mut now.rumors {
        sort_unique(&mut rumor.fact_ids);
        sort_unique(&mut rumor.known_by);
    }
    now.messages.sort_by(|a, b| a.id.cmp(&b.id));
    for message in &mut now.messages {
        sort_unique(&mut message.to);
        sort_unique(&mut message.fact_ids);
    }
    let occupancy = rebuilt_occupancy(now);
    for (place_id, place) in &mut now.places {
        sort_unique(&mut place.notes);
        place.actors = occupancy.get(place_id).cloned().unwrap_or_default();
    }
    for actor in now.actors.values_mut() {
        sort_unique(actor.flags_mut());
        sort_unique(actor.known_facts_mut());
    }
    for clock in now.clocks.values_mut() {
        sort_unique(&mut clock.owners);
    }
}

fn rebuilt_occupancy(now: &Now) -> std::collections::BTreeMap<String, Vec<String>> {
    let mut out: std::collections::BTreeMap<String, Vec<String>> = now
        .places
        .keys()
        .map(|id| (id.clone(), Vec::new()))
        .collect();
    for (actor_id, actor) in &now.actors {
        if let Some(list) = out.get_mut(actor.place()) {
            list.push(actor_id.clone());
        }
    }
    for list in out.values_mut() {
        list.sort();
    }
    out
}

fn check_occupancy(now: &Now) -> Result<(), Code> {
    for actor in now.actors.values() {
        if !now.places.contains_key(actor.place()) {
            return Err(Code::Schema);
        }
    }
    let expected = rebuilt_occupancy(now);
    for (place_id, place) in &now.places {
        let mut listed = place.actors.clone();
        listed.sort();
        let got = expected.get(place_id).map(Vec::as_slice).unwrap_or(&[]);
        if listed != got {
            return Err(Code::Schema);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn docks_now_round_trips_through_emitter() {
        let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../packs/oneshot-docks/state/now.yaml");
        let original = fs::read_to_string(&path).unwrap();
        let now = load_now(&original).unwrap();
        let emitted = emit_now(&now).unwrap();
        assert_eq!(emitted, original);
    }
}
