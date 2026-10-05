use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Serialize)]
struct Sample {
    schema: &'static str,
    last_tick_at: &'static str,
    flag: bool,
    empty_map: BTreeMap<&'static str, &'static str>,
    names: Vec<&'static str>,
    plain: &'static str,
    yes_word: &'static str,
    num_word: &'static str,
}

const SAMPLE: &str = "\
schema: now/v0
last_tick_at: \"2026-01-01T00:00:00Z\"
flag: false
empty_map: {}
names:
  - aldric
  - mira
plain: Oil is on the shelf.
yes_word: \"yes\"
num_word: \"1842\"
";

pub(crate) fn now_sample_emits_the_recorded_bytes() {
    let sample = Sample {
        schema: "now/v0",
        last_tick_at: "2026-01-01T00:00:00Z",
        flag: false,
        empty_map: BTreeMap::new(),
        names: vec!["aldric", "mira"],
        plain: "Oil is on the shelf.",
        yes_word: "yes",
        num_word: "1842",
    };
    let text =
        serde_saphyr::to_string_with_options(&sample, spookery_kernel::codec::NOW_EMIT_OPTIONS())
            .unwrap_or_else(|err| panic!("emit sample: {err}"));
    assert_eq!(
        text.len(),
        166,
        "emitter sample is {} bytes:\n{text:?}",
        text.len()
    );
    assert_eq!(text, SAMPLE);
}
