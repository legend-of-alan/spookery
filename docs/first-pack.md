# Write your first pack

`spook new <dir>` writes a small pack and prints the next three commands. This page edits that pack. Each change is a fenced block the test can apply. A larger example is `packs/sample-d20`, a short fight with a fixed-damage strike.

## Start

```sh
spook new /tmp/mypack
```

The pack is a waystation with two places, so travel has somewhere to go.

- `yard` is the start place. `lane` is the other place.
- `pc_ada` is the player character. The cast file `cast/pc-ada.md` is a `pc_template`, and `state/save.yaml` has a live `pc_ada` actor. A template alone does not sit a player.
- `keeper` is an NPC in the lane. One habit, `tend_the_yard`, runs while the table is empty and sets the flag `tended`.

`rules/meters.yaml` defines the meter the check uses:

```yaml
schema: meters
meters:
  - id: nerve
    min: 0
    base_cap: 100
    max: 110
    start: 1
```

`min` is the floor, `base_cap` is the trained cap, and `max` is as high as current can go. `start` is the base a new actor stores. Current is base plus offset, and it is not a field in the save.

`rules/checks.yaml` rolls that meter:

```yaml
schema: checks
checks:
  - id: nerve
    dice:
      count: 2
      sides: 6
    stat: nerve
```

`rules/verbs.yaml` has `wait`, `travel`, and `steady`. `wait` is a yield: `do: []` and no success, partial, or failure list. `travel` takes a place. `steady` names the `nerve` check. `tend_the_yard` is a habit rule on `cast/keeper.md`, with `when: habit`.

`rules/tick.yaml` uses the same counters as the docks pack and sets `idle: on`, so an empty table still wakes.

`state/save.yaml` is the save. `state_rev` starts at 0. `pack_rev` is the hash of `world/`, `cast/`, and `rules/` only. `spook new` computes it. Leave that line for `spook migrate` when the canon changes.

## Add a place

A place is a Markdown file under `world/places/`. The header is the frontmatter. The prose after the closing `---` is what a viewer can read when the fact is public.

```yaml path=world/places/porch.md mode=create
---
schema: place
id: porch
facts:
  - id: fact_porch_dry
    text: The porch is dry.
    hidden: false
---
The porch faces the yard.
```

The save lists the places people can stand in. Add `porch` under `places` in `state/save.yaml`.

```yaml path=state/save.yaml mode=insert-before anchor=actors:
  porch:
    items: {}
    notes: []
    actors: []
```

`porch` is not on anyone's `range` yet. `travel porch` stays put until you add that id to the character's `tick.range`.

## Add a player move

Append a yield to `rules/verbs.yaml`. A yield commits a tick and changes no numbers.

```yaml path=rules/verbs.yaml mode=append
  - id: listen
    when:
      verb: listen
    do: []
```

`say` is speech, not a rule. Do not add a rule named `say`.

## Add a check with partial_at

`partial_at` is an absolute total, strictly below the roll `target`. The rule needs `check` and a `partial` list. A total at or above the target takes `success`. A total at or above `partial_at` and below the target takes `partial`. A lower total is a miss. This rule reuses the `nerve` check. The target is 8 and `partial_at` is 4, so a 2d6 plus the nerve meter can land in either band.

```yaml path=rules/verbs.yaml mode=append
  - id: press
    when:
      verb: press
    roll:
      check: nerve
      target: 8
      partial_at: 4
    success:
      - note:
          id: note_press
          to:
            - pc_ada
          text: The press holds.
    partial:
      - note:
          id: note_press_part
          to:
            - pc_ada
          text: The press slips.
```

## Give the NPC a second habit

A habit is a rule on the cast file with `when: habit`. The cast file keeps a `# more-habits` line for this insert. The first habit whose `if` holds is the one that runs. This one leaves a note. It does not run while `tend_the_yard` is ahead of it and ready.

```yaml path=cast/keeper.md mode=insert-before anchor=  # more-habits
  - id: light_the_lamp
    when: habit
    do:
      - note:
          id: note_lamp
          to:
            - pc_ada
          place: yard
          fact_ids: []
          text: The lamp is lit.
```

## Check, migrate, and play

The canon changed, so `pack_rev` no longer matches. The save edit is also not the canonical text until migrate rewrites it. Stop `spook run` before migrate. The server keeps the save in memory.

```sh
spook check --json /tmp/mypack
spook migrate /tmp/mypack
spook check --json /tmp/mypack
```

The first check prints a JSON array and exits 1. After migrate, the check prints `[]` and exits 0.

```sh
spook run /tmp/mypack
```

```sh
spook play /tmp/mypack
```

You sit `pc_ada`. The first picture is the yard. `travel lane` leaves for the lane and commits a tick, because no GM is seated. `listen`, `wait`, `steady`, and `press` are the other player verbs.
