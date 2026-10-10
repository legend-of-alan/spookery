---
name: author-pack
description: >
  Edit a Spookery pack and check it with spook check --json. Use when
  asked to author a pack, add a move, meter, check, or habit, or make
  a pack pass spook check.
---

# Author a pack

You have the `spook` CLI and a pack directory. The keys for the canon files are listed below. A key that is not listed is rejected. Follow the `hint` on each diagnostic.

## What a pack is

A pack is a directory. Copy it before you edit. `spook run` writes `state/` (the snapshot, the tape, and lock files) into the pack you pass. Leave `packs/oneshot-docks` as it is and edit the copy.

- `pack.yaml` has `schema: pack`, `id`, `title`, `system`, and `start` with `place` and `pc`.
- `world/places/*.md` is a place. The frontmatter has `schema: place`, `id`, and `facts`. Prose comes after the closing `---`.
- `cast/*.md` is a person. The frontmatter has `schema: cast`, `id`, `kind` (`npc` or `pc_template`), `facts`, and an optional `tick` block. Prose comes after the closing `---`.
- `rules/moves.yaml` has `schema: moves` and a `moves` list.
- `rules/checks.yaml` has `schema: checks` and a `checks` list.
- `rules/meters.yaml` has `schema: meters` and a `meters` list.
- `rules/tick.yaml` has `schema: tick`, the counters below, and `pause.named_plot_clocks`.
- `state/now.yaml` has `schema: now`. It is the save. `spook check` reads it when it is present. `spook run` and `spook migrate` require it. Leave the keys already in that file in place. This skill does not list them.

Each file's format tag is the unversioned word after `schema:`: `pack`, `place`, `cast`, `meters`, `moves`, `checks`, `tick`, or `now`. A version suffix is rejected. Unknown fields are rejected. Ids are not empty.

`pack_rev` in `state/now.yaml` is the hash of `world/`, `cast/`, and `rules/` only. `spook migrate` is what stores a new hash.

A fact has `id`, `text`, and `hidden` (`true` or `false`).

## Rows

A row in `rules/moves.yaml` has:

- `name`, required. `say` is speech, not a row.
- `audience`, required: `player` or `habit`. Omitting it, or any other word, is rejected.
- `token`, required: `none`, `actor`, or `place`.
- `on`, required: `speaker` or `named`.
- The legal pairs are `none` with `speaker`, `actor` with `named`, and `place` with `speaker`.
- `effects`: templates that run on both outcomes. `effects: []` with no `success` and no `failure` is a yield. `wait` is a yield.
- `success` and `failure`: optional lists for that outcome. An empty list is rejected.
- `check`: optional id that must exist in `rules/checks.yaml`.
- `difficulty`: optional integer. The default is 0. It is used when `check` is set.
- `item`: optional id of an item the actor already holds. An empty id is rejected.

A duplicate name is rejected. A habit that names a `player` row is rejected. A habit row name has to be an `audience: habit` move.

`class` is one of `flag`, `location`, `meter`, `stock`, `clock`, `message`, or `memory`. A field that does not belong on that class is rejected.

- `flag` has `key` and `value` (`true` or `false`). The key `busy` is rejected. The row's token is `none` or `actor`.
- `location` has `to` set to `token` or a place id. `to: token` needs `token: place`. Omitting `to` is legal when `token` is `place`. `from` is the actor's live place and is not a field.
- `meter` has `key` (a meter id) and `write` (`base`, `offset`, or `snap`). `delta` is an integer, except `write: snap` has no `delta`.
- `stock` has `place`, `sku`, and `delta`.
- `clock` has `clock` and `delta`.
- `message` has `id`, `to` (a list of one or more actor ids), and `text`. `place` and `fact_ids` may be omitted.
- `memory` has `fact_ids` and `tags` (each list may be empty). `text` may be omitted.

These words are not ids: `token`, `here`, `first_rate`, `rate`, `first_drift`, `toward_target`, `first_known`, `first_listener`, `keep`, `first_due_note`, `first_owned`.

## Meters

A meter has `id`, `min`, `base_cap`, `max`, and `start`. `min` is at or below `base_cap`, `base_cap` is at or below `max`, and `start` lies between `min` and `base_cap` inclusive. The save stores `base` and `offset` on the actor. Current is derived and is not a field.

## Checks

A check has `id`, `dice.count`, `dice.sides`, and `stat`. `count` and `sides` are integers of at least 1. `stat` is a meter id. A row's `check` names one of these ids. A check's `stat` names a meter.

## Habits

A habit is on a cast member's `tick` block. `rules/tick.yaml` is the budget, not the habit list.

The tick block has required `eligible` (`true` or `false`). Optional keys are `range` (place ids; an unknown id is rejected), `stock_rate` (a place, then a sku, then an integer), `flags` (a list of names), and `habits`.

A habit has:

- `id`, required and unique in that file.
- `runs_while_empty`, required boolean.
- `rows`, required, at least one move name, each an `audience: habit` row.
- `from_hour` and `to_hour` both set, or neither. Each is an integer in `0..24`.
- `clock` and `at_least` both set, or neither.
- `unless_message`, optional message id.

`rules/tick.yaml` requires `schema: tick` and `pause.named_plot_clocks` (`pause_when_empty` or `always_run`). These counters may be omitted: `max_intents_applied`, `max_npcs_acting`, `max_effects_per_intent`, `max_state_keys_mutated`, `max_memory_writes`, `idle_acts_per_world_day`, `live_acts_per_world_hour`, `tick_every_real_seconds`, `live_tick_every_real_seconds`, `idle_minutes_per_tick`, `live_minutes_per_tick`, `idle_catchup_max_ticks`, `idle_max_world_hours_per_real_hour`. A counter below zero is rejected.

## The loop

1. Edit the copy.
2. Run `spook check --json <pack>`.
3. Read the JSON array on stdout. Each object has `file`, `line`, `column`, `path`, `code`, `message`, and `hint`. Null means unknown. `line` and `column` are 1-based when present. Fix every object. Use `code` and `hint`. The message text may change.
4. Repeat until stdout is `[]` and the process exits 0. Any problem exits 1. In `--json` mode nothing else is written to stdout. A missing argument is clap's usage text on stderr and exit 2.
5. Run `spook status <pack>`. It prints the host digest.

Leave `state/now.yaml` for `spook migrate` to rewrite. The codes are `SCHEMA`, `NOT_IN_SPEC`, `PACK_REV_MISMATCH`, `HAND_EDIT`, and `HIDDEN_FACT_IN_BODY`.

- `SCHEMA` is a shape error. Change the file the diagnostic names.
- `NOT_IN_SPEC` means a required file is missing. Add that file.
- `PACK_REV_MISMATCH` means `pack_rev` does not match `world/`, `cast/`, and `rules/`.
- `HAND_EDIT` means `state/now.yaml` is not the canonical save text.
- `HIDDEN_FACT_IN_BODY` means a hidden fact's text is in a Markdown body. Remove that sentence, or set `hidden: false`.

`spook migrate <pack>` stores the current hash, keeps a hand edit of the save, and rewrites `state/now.yaml`. Run it when the code is `PACK_REV_MISMATCH` or `HAND_EDIT`, then check again. Migrate does not fix a shape error.

## Worked example

On a copy of `packs/oneshot-docks`, append this row to `rules/moves.yaml`:

```yaml
  - name: scan_the_street
    audience: player
    token: none
    on: speaker
    effects: []
```

`spook check --json <pack>` reports `PACK_REV_MISMATCH` on `state/now.yaml`, because the rules changed and the save still has the old hash. Run `spook migrate <pack>`, then `spook check --json <pack>`. stdout is `[]`. Then run `spook status <pack>`.
