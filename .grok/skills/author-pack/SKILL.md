---
name: author-pack
description: >
  Edit a Spookery pack and check it with spook check --json. Use when
  asked to author a pack, add a move, meter, check, or habit, or make
  a pack pass spook check.
---

# Author a pack

Start a pack with `spook new <dir>`. The walkthrough is `docs/first-pack.md`.

You have the `spook` CLI and a pack directory. The keys for the canon files are listed below. A key that is not listed is rejected. Follow the `hint` on each diagnostic.

## What a pack is

A pack is a directory. Copy it before you edit. `spook run` writes `state/` (the snapshot, the tape, and lock files) into the pack you pass. Leave `packs/oneshot-docks` as it is and edit the copy.

- `pack.yaml` has `schema: pack`, `id`, `title`, `system`, and `start` with `place` and `pc`.
- `world/places/*.md` is a place. The frontmatter has `schema: place`, `id`, and `facts`. Prose comes after the closing `---`.
- `cast/*.md` is a person. The frontmatter has `schema: cast`, `id`, `kind` (`npc` or `pc_template`), `facts`, and an optional `tick` block. Prose comes after the closing `---`.
- `rules/verbs.yaml` has `schema: verbs` and a `rules` list. Place, item, cast, and story files may hold the same rule shape.
- `rules/checks.yaml` has `schema: checks` and a `checks` list.
- `rules/meters.yaml` has `schema: meters` and a `meters` list.
- `rules/tick.yaml` has `schema: tick`, the counters below, and `pause.named_plot_clocks`.
- `state/save.yaml` has `schema: now`. It is the save. `spook check` reads it when it is present. `spook run` and `spook migrate` require it. Leave the keys already in that file in place. This skill does not list them.

Each file's format tag is the unversioned word after `schema:`: `pack`, `place`, `cast`, `meters`, `verbs`, `checks`, `tick`, `item`, `story`, or `now`. A version suffix is rejected. Unknown fields are rejected. Ids are not empty. `rules/moves.yaml` is rejected. Write rules in `rules/verbs.yaml` or on the owner.

`pack_rev` in `state/save.yaml` is the hash of `world/`, `cast/`, and `rules/` only. `spook migrate` is what stores a new hash.

A fact has `id`, `text`, and `hidden` (`true` or `false`).

## Rules

A rule has:

- `id`, required. `say` is speech, not a rule.
- `when`, required. A verb is `when: {verb, target}`. `target` is `none`, `actor`, `place`, `exit`, or `item`. A habit is `when: habit`.
- `if`: a list. One condition under each dash. `has_item` names `who` and `id`.
- `do`: effects that run on every outcome. `do: []` with no `success`, `partial`, or `failure` is a yield. `wait` is a yield.
- `roll`: optional `check`, `target`, and `partial_at`. `target` is a number or `{meter: {who, key}, plus}`. `partial_at` is strictly below `target` and needs a `partial` list.
- `success`, `partial`, and `failure`: the band lists. `failure` is the miss list.
- `cost`: optional `time`, `meter`, and `items`. A cost the actor cannot pay changes nothing.

A duplicate id is rejected. Two verb rules for the same verb in one file are rejected.

One effect key under each dash: `flag`, `location`, `meter`, `stock`, `clock`, `note`, `narrate`, `memory`, `item`, or `reveal`.

- `flag` has `who`, `key`, and `value` (`true` or `false`). The key `busy` is rejected.
- `location` has `to: target` or a place id.
- `meter` has `who`, `key`, and `add` or `set`.
- `stock` has `place`, `sku`, and `add`.
- `clock` has `id` and `add` or `start`.
- `note` has `id`, `to` (a list of one or more actor ids), and `text`. `place` and `fact_ids` may be omitted. The save still stores these under `messages`.
- `narrate` has `to` and `text`. It is not stored.
- `memory` has `who`. `fact_ids` and `text` may be omitted.

These words are not a `who`: `token`, `first_rate`, `rate`, `first_drift`, `toward_target`, `first_known`, `first_listener`, `keep`, `first_due_note`, `first_owned`. `self`, `target`, `subject`, `here`, and `world` are.

## Meters

A meter has `id`, `min`, `base_cap`, `max`, and `start`. `min` is at or below `base_cap`, `base_cap` is at or below `max`, and `start` lies between `min` and `base_cap` inclusive. The save stores `base` and `offset` on the actor. Current is derived and is not a field.

## Checks

A check has `id`, `dice.count`, `dice.sides`, and `stat`. `count` and `sides` are integers of at least 1. `stat` is a meter id. A rule's `roll.check` names one of these ids. A check's `stat` names a meter.

## Habits

A habit is a rule with `when: habit` on that cast member. `rules/tick.yaml` is the budget, not the habit list.

The tick block has required `eligible` (`true` or `false`). Optional keys are `range` (place ids; an unknown id is rejected), `stock_rate` (a place, then a sku, then an integer), and `flags` (a list of names).

The runner takes the first habit whose `if` holds. `hour.from` and `hour.to` are integers in `0..24` and must differ. `humans` compares how many people are seated. `note_exists` is off once that note is in the save. A habit with no `humans` condition can run while the table is empty.

`rules/tick.yaml` requires `schema: tick` and `pause.named_plot_clocks` (`pause_when_empty` or `always_run`). These counters may be omitted: `max_intents_applied`, `max_npcs_acting`, `max_effects_per_intent`, `max_state_keys_mutated`, `max_memory_writes`, `idle_acts_per_world_day`, `live_acts_per_world_hour`, `tick_every_real_seconds`, `live_tick_every_real_seconds`, `idle_minutes_per_tick`, `live_minutes_per_tick`, `idle_catchup_max_ticks`, `idle_max_world_hours_per_real_hour`. A counter below zero is rejected.

## The loop

1. Edit the copy.
2. Run `spook check --json <pack>`.
3. Read the JSON array on stdout. Each object has `file`, `line`, `column`, `path`, `code`, `message`, and `hint`. Null means unknown. `line` and `column` are 1-based when present. Fix every object. Use `code` and `hint`. The message text may change.
4. Repeat until stdout is `[]` and the process exits 0. Any problem exits 1. In `--json` mode nothing else is written to stdout. A missing argument is clap's usage text on stderr and exit 2.
5. Run `spook status <pack>`. It prints the host digest.

Leave `state/save.yaml` for `spook migrate` to rewrite. The codes are `SCHEMA`, `NOT_IN_SPEC`, `PACK_REV_MISMATCH`, `HAND_EDIT`, and `HIDDEN_FACT_IN_BODY`.

- `SCHEMA` is a shape error. Change the file the diagnostic names.
- `NOT_IN_SPEC` means a required file is missing. Add that file.
- `PACK_REV_MISMATCH` means `pack_rev` does not match `world/`, `cast/`, and `rules/`.
- `HAND_EDIT` means `state/save.yaml` is not the canonical save text.
- `HIDDEN_FACT_IN_BODY` means a hidden fact's text is in a Markdown body. Remove that sentence, or set `hidden: false`.

`spook migrate <pack>` stores the current hash, keeps a hand edit of the save, and rewrites `state/save.yaml`. Run it when the code is `PACK_REV_MISMATCH` or `HAND_EDIT`, then check again. Migrate does not fix a shape error.

## Worked example

On a copy of `packs/oneshot-docks`, append this rule to `rules/verbs.yaml`:

```yaml
  - id: scan_the_street
    when:
      verb: scan_the_street
    do: []
```

`spook check --json <pack>` reports `PACK_REV_MISMATCH` on `state/save.yaml`, because the rules changed and the save still has the old hash. Run `spook migrate <pack>`, then `spook check --json <pack>`. stdout is `[]`. Then run `spook status <pack>`.
