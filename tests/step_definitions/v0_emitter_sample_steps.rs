use std::collections::BTreeMap;

use cucumber::{then, when};
use serde::Serialize;

use crate::World;

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

#[when(expr = "the now emitter sample is rendered")]
fn render_sample(w: &mut World) {
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
    w.text =
        serde_saphyr::to_string_with_options(&sample, spookery_kernel::codec::NOW_EMIT_OPTIONS())
            .unwrap_or_else(|err| panic!("emit sample: {err}"));
}

#[then(expr = "the emitter sample is {int} bytes")]
fn sample_len(w: &mut World, n: i64) {
    assert_eq!(
        w.text.len(),
        n as usize,
        "emitter sample is {} bytes:\n{:?}",
        w.text.len(),
        w.text
    );
}

#[then(expr = "the emitter sample equals the canon text")]
fn sample_text(w: &mut World) {
    assert_eq!(w.text, SAMPLE);
}
