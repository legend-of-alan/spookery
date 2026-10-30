# Pack reference

Reference for people and agents writing a pack. The walkthrough is [Write your first pack](first-pack.md). What the engine promises is [Engine guarantees](engine-guarantees.md).

`spook check <pack>` is the authority for keys. An unknown key is rejected. `spook check --json <pack>` prints one array of `file`, `line`, `column`, `path`, `code`, `message`, and `hint`. The fences below are one pack, and that check accepts them. A rule that names `oil`, `knife`, or `nerve` uses the item and meter files on this page.

The first key in each file is `schema`, with no version suffix. In a Markdown file that key sits in the frontmatter.

## Layout

| Path | What it holds |
|---|---|
| `pack.yaml` | Id, title, system, and the start place and PC. Tag `pack`. |
| `rules/verbs.yaml` | Shared verb rules. Tag `verbs`. |
| `rules/meters.yaml` | Meter definitions. Tag `meters`. |
| `rules/checks.yaml` | Check definitions. Tag `checks`. |
| `rules/budget.yaml` | Round budgets and the idle policy. Tag `budget`. |
| `world/places/<id>.md` | One place: facts, exits, and that place's rules. |
| `world/items/<id>.md` | One item: name, tags, prose, and that item's rules. |
| `cast/<name>.md` | One NPC or PC template, and that cast member's rules. |
| `story/<id>.yaml` | Rules owned by a story. Tag `story`. |
| `state/save.yaml` | The live world. It is the save, not canon. |

Check rejects `rules/moves.yaml`. Write those rules in `rules/verbs.yaml` or in the owner's file.

A rule's owner is the file it lives in. The full id is `verb:<id>`, `place:<place>/<id>`, `item:<item>/<id>`, `cast:<actor>/<id>`, or `story:<story>/<id>`. `spook check --rules <pack>` prints those ids when the pack has no problems.

For `attack`, a wielded weapon's rule wins when that item has one. A verb uses the target item's rule, then the target actor's, then the rule on the current place, then `rules/verbs.yaml`. When no rule matches, the move is refused before the round with `NOT_IN_SPEC`, and the turn is not spent. A word is a verb only when some rule in the pack names it, so `use` is a verb only in a pack where some rule says `verb: use`. A habit runs only for the NPC whose cast file holds it.

`pack_rev` in the save is the hash of `world/`, `cast/`, and `rules/`. After those files change, `spook migrate <pack>` stores the new hash. Migrate does not fix a shape error.

## Pack

`start.place` is the root of the exit graph. `start.pc` is the PC sat when `spook play` is given no `--as`. `checkpoint_every` is optional. It is the number of rounds between journal checkpoints. The default is 100. Zero is rejected. The field is not part of the canon hash.

```yaml file=pack.yaml
schema: pack
id: reference
title: Reference
system: reference
start:
  place: yard
  pc: pc_ada
```

## Places and exits

A place file is Markdown. The header between the `---` lines is the frontmatter. The prose after the closing `---` is the body a viewer can read. `display_name` is required. It is the name a player sees. The id is not turned into a title.

An exit has `to` and `name`. It may have `cost`, `if`, `locked_text`, and `hidden` together with `revealed_by`. `to` is a place id. Two exits on one place with the same name are rejected. A hidden exit needs `revealed_by`, and that value is a fact id from this pack. A place no path reaches from the start, counting hidden and locked exits, is a warning.

`cost.time` is `30s`, `10m`, `2h`, or `1d`, stored as minutes. Seconds round up to the next minute. `if` uses the same conditions as a rule. When that `if` fails, the exit is locked: the engine shows `locked_text` and changes nothing.

The traveller stays at the origin until the journey finishes. The busy time is the rule's minutes plus the exit's minutes.

```yaml file=world/places/yard.md
---
schema: place
id: yard
display_name: Yard
facts:
  - id: fact_yard_open
    text: The yard gate stands open.
    hidden: false
  - id: fact_hidden_stair
    text: Ivy hides a stair.
    hidden: true
exits:
  - to: lane
    name: lane
    cost:
      time: 10m
  - to: loft
    name: loft
    hidden: true
    revealed_by: fact_hidden_stair
  - to: cellar
    name: cellar
    if:
      - has_item:
          who: self
          id: oil
          count: 1
    locked_text: The cellar door is locked.
rules:
  - id: bell
    when:
      verb: bell
    cost:
      time: 1d
    do: []
  - id: step_through
    when:
      verb: step_through
    do:
      - location:
          to: lane
          teleport: true
---
Gravel underfoot.
```

```yaml file=world/places/lane.md
---
schema: place
id: lane
display_name: Lane
facts:
  - id: fact_lane_empty
    text: The lane is empty.
    hidden: false
exits:
  - to: yard
    name: yard
---
The lane runs past the yard.
```

```yaml file=world/places/loft.md
---
schema: place
id: loft
display_name: Loft
facts:
  - id: fact_loft_rope
    text: The loft smells of rope.
    hidden: false
exits:
  - to: yard
    name: yard
---
Rope, and a view of the yard.
```

```yaml file=world/places/cellar.md
---
schema: place
id: cellar
display_name: Cellar
facts:
  - id: fact_cellar_dark
    text: The cellar is dark.
    hidden: false
exits:
  - to: yard
    name: yard
---
Stone, and a low door.
```

## Items

An item file is Markdown with the same frontmatter shape. `tags` may include `weapon`. Wielding an item without that tag is refused when the move is played. The file name is not the id.

```yaml file=world/items/oil.md
---
schema: item
id: oil
name: Oil
tags: []
rules:
  - id: use
    when:
      verb: use
      target: item
    do:
      - item:
          who: self
          id: oil
          add: -1
---
A tin of oil.
```

```yaml file=world/items/knife.md
---
schema: item
id: knife
name: Knife
tags:
  - weapon
rules: []
---
A short knife.
```

## Cast

`kind` is `npc` or `pc_template`. A template does not sit a player. The save does. The file name is not the id: `cast/pc-ada.md` has id `pc_ada`. `display_name` is required, and it is the name a player sees.

`idle.eligible` says whether the idle runner may pick this NPC. `when: habit` is a habit of that NPC. The runner takes the first habit whose `if` holds and whose `do` list expands.

```yaml file=cast/pc-ada.md
---
schema: cast
id: pc_ada
kind: pc_template
display_name: Ada
facts: []
idle:
  eligible: false
  flags: []
---
Ada keeps the yard.
```

```yaml file=cast/keeper.md
---
schema: cast
id: keeper
kind: npc
display_name: Keeper
facts:
  - id: fact_ledger_short
    text: The oil ledger is one tin short.
    hidden: true
idle:
  eligible: true
  flags: []
rules:
  - id: tend
    when: habit
    if:
      - all:
          - hour:
              from: 8
              to: 16
          - humans:
              op: "=="
              value: 0
          - meter:
              who: self
              key: nerve
              op: ">="
              value: 1
      - any:
          - flag:
              who: self
              key: tended
              is: false
          - knows:
              who: pc_ada
              fact: fact_yard_open
      - not:
          note_exists: note_tended
      - has_item:
          who: self
          id: oil
          count: 1
      - wielding:
          who: self
          id: knife
      - at:
          who: here
          place: yard
      - clock:
          id: shop_day
          op: "<"
          value: 3
      - humans:
          op: "<="
          value: 3
      - meter:
          who: pc_ada
          key: nerve
          op: ">"
          value:
            meter:
              who: self
              key: nerve
            plus: 1
    do:
      - flag:
          who: self
          key: tended
          value: true
      - note:
          id: note_tended
          to:
            - pc_ada
          place: yard
          fact_ids: []
          text: The yard is swept.
---
Keeper sweeps the yard.
```

## Story

A story file is YAML. Its rules take the id `story:<id>/<local>`. Verb resolution does not read them.

```yaml file=story/samples.yaml
schema: story
id: samples
rules:
  - id: mark
    when:
      verb: mark
    do: []
  - id: spent
    when:
      change:
        meter: nerve
        crosses: "<=0"
    do:
      - flag:
          who: self
          key: spent
          value: true
```

## Rules

A rule has `id`, `when`, and `do`. It may have `if`, `cost`, `roll`, `success`, `partial`, and `failure`.

`when` is `habit`, a map with `verb` and `target`, or a map with `change`. `target` is `none`, `actor`, `place`, `exit`, or `item`. An omitted `target` means `none`. `say` is speech, so that id is not a rule and not a verb. `when` has one of `verb` or `change`, not both.

A change rule fires when its watch becomes true. It does not fire again while the watch stays true. What the engine does with that firing is in [Engine guarantees](engine-guarantees.md). `story/samples.yaml` has a meter watch. The other watches are:

| Watch | Fields |
|---|---|
| Arrive | `arrive` is a place id, or `here` on a place rule. `who` is `any` (the default), `pc`, `npc`, or an actor id. |
| Leave | `leave` and `who`, same as arrive. |
| Meter | `meter` is a meter id. `crosses` is an operator and a number, such as `<=0`. |
| Flag | `flag` is the flag. `becomes` is `true` or `false`. |
| Clock | `clock` is a clock id. The watch is that clock reaching its max. |
| Item | `item` is an item id, with `gained: true` or `lost: true`. |
| Knows | `knows` is a fact id. The watch is that fact becoming known. |

A change rule has no `cost`. Check rejects one. It may have `if`, `roll`, and the outcome lists. `if` is read after the change.

Matching change rules run in owner order (shared verbs, then place, item, cast, story), then by file path, then by position in the file. They run again on the changes they just wrote, until a pass matches nothing. In one pass a rule runs at most once for each subject. `max_trigger_depth` in `rules/budget.yaml` caps the passes. Omitted, it is 4. The next pass is recorded and does not run. Check warns when change rules set each other's watched values. A `narrate` or `note` on a change rule that can reach a viewer who cannot see that change is `TRIGGER_LEAK`.

`if` is a list. Each dash is one condition. `all` is and, `any` is or, and `not` negates one condition. Two conditions under one dash are rejected. `who` is `self`, `target`, `here`, `world`, `place:<id>`, or an actor id. On `has_item`, `wielding`, and an item effect, `id: target` is the intent's target. The habit on `cast/keeper.md` uses each condition. `op` is `<`, `<=`, `==`, `>=`, or `>`. `hour.from` and `hour.to` are integers in `0..24` and they differ.

`cost` may name `time`, `meter`, and `items`. `time` uses the duration words from exits. A cost the actor cannot pay is refused when the move is played, and the world stays as it was.

`roll` names a `check` and a `target`. `partial_at` is an absolute total, strictly below a literal target, and it needs a `partial` list. `do` runs on every band. `success`, `partial`, and `failure` run on that band. `failure` is the miss. A yield is `do: []` with no outcome lists.

A number is an integer or an [expression](spec.md). A meter plus an integer still loads.

The flag key `busy` is rejected. An actor is busy while a journey's finish minute is still ahead of world time.

```yaml file=rules/verbs.yaml
schema: verbs
rules:
  - id: wait
    when:
      verb: wait
      target: none
    do: []

  - id: travel
    when:
      verb: travel
      target: exit
    cost:
      time: 2h
    do:
      - location:
          via: target

  - id: greet
    when:
      verb: greet
      target: actor
    do: []

  - id: visit
    when:
      verb: visit
      target: place
    do: []

  - id: take
    when:
      verb: take
      target: item
    do:
      - item:
          who: here
          id: target
          add: -1
      - item:
          who: self
          id: target
          add: 1

  - id: press
    when:
      verb: press
    cost:
      time: 30s
      meter:
        nerve: 1
      items:
        oil: 1
    roll:
      check: nerve
      target: "6 + 2"
      partial_at: 4
    do:
      - narrate:
          to: pc_ada
          text: You set your weight.
          fact_ids: []
    success:
      - flag:
          who: self
          key: pressed
          value: true
    partial:
      - flag:
          who: self
          key: pressed
          value: false
    failure:
      - narrate:
          to: self
          text: The press slips.
          fact_ids: []

  - id: meter_add
    when:
      verb: meter_add
    do:
      - meter:
          who: self
          key: nerve
          add: 1

  - id: meter_set
    when:
      verb: meter_set
    do:
      - meter:
          who: target
          key: nerve
          set: 3

  - id: shrug
    when:
      verb: shrug
    do:
      - meter:
          who: self
          key: nerve
          add: -1
          resist:
            who: target
            check: nerve
            target: 4
            resisted:
              - narrate:
                  to: self
                  text: The hit glances off.
                  fact_ids: []
      - flag:
          who: self
          key: shrugged
          value: true
          resist: same

  - id: leave_note
    when:
      verb: leave_note
    do:
      - note:
          id: note_lane
          to:
            - pc_ada
          place: yard
          fact_ids: []
          text: A chalk mark points down the lane.

  - id: tell
    when:
      verb: tell
    do:
      - narrate:
          to: here
          text: A bell rings once.
          fact_ids: []

  - id: advance_day
    when:
      verb: advance_day
    do:
      - clock:
          id: shop_day
          add: 1
          start: true

  - id: remember
    when:
      verb: remember
    do:
      - memory:
          who: self
          fact_ids:
            - fact_yard_open
          text: The gate was open at dawn.

  - id: give_oil
    when:
      verb: give_oil
    do:
      - item:
          who: here
          id: oil
          add: 1
          max: 4
          resist:
            who: world
            check: nerve
            target:
              meter:
                who: self
                key: nerve
              plus: 2
            resisted: []

  - id: show_stair
    when:
      verb: show_stair
    do:
      - reveal:
          who: place:yard
          fact: fact_hidden_stair

  - id: arm
    when:
      verb: arm
    do:
      - wield:
          who: self
          id: knife

  - id: disarm
    when:
      verb: disarm
    do:
      - unwield:
          who: keeper
```

`travel` follows an exit of the current place. `via: target` is the exit the player named.

## Meters

`min` is the floor, `base_cap` is the trained cap, and `max` is as high as current can go. `start` is the base a new meter takes on every existing actor when `spook migrate` runs. A save that lacks a meter the rules define, with no migrate to add it, is refused. Current is base plus offset, and it is not stored. `hidden: true` keeps that meter off the player's view. Leave it out and the player sees the meter.

```yaml file=rules/meters.yaml
schema: meters
meters:
  - id: nerve
    min: 0
    base_cap: 100
    max: 110
    start: 1
```

## Checks

`stat` is a meter id. `dice.count` and `dice.sides` are the check's dice. A rule's target and partial band are expressions.

```yaml file=rules/checks.yaml
schema: checks
checks:
  - id: nerve
    dice:
      count: 2
      sides: 6
    stat: nerve
```

## Budget

`idle` is `on` or `off`. Omitted means `on`. `pause.named_plot_clocks` is `pause_when_empty` or `always_run`. Omitted counters use the engine defaults. `round_every_real_seconds` is real seconds between wakes, `idle_minutes_per_round` and `live_minutes_per_round` are world minutes one round writes, `idle_catchup_max_rounds` is the most rounds one catch-up runs, and `idle_max_world_hours_per_real_hour` is the world hours one real hour of absence may write. The file below sets them.

```yaml file=rules/budget.yaml
schema: budget
idle: on
max_intents_applied: 12
max_npcs_acting: 8
max_effects_per_intent: 4
max_state_keys_mutated: 24
max_memory_writes: 8
idle_acts_per_world_day: 3
live_acts_per_world_hour: 6
round_every_real_seconds: 300
idle_minutes_per_round: 60
live_minutes_per_round: 5
idle_catchup_max_rounds: 12
idle_max_world_hours_per_real_hour: 8
max_fastforward_rounds: 12
max_trigger_depth: 4
pause:
  named_plot_clocks: pause_when_empty
```

## Effects

One effect under each dash. The rules in `rules/verbs.yaml` are the examples. `step_through` on the yard is the `location.to` example.

`resist` on `meter`, `flag`, `item`, or `narrate` names `who`, `check`, `target`, and `resisted`. `resist: same` reuses the previous resist roll in that list. `do` counts for the lists after it. Check rejects `resist: same` when no earlier effect in that run has rolled a resist.

`location.via` names an exit from the actor's current place. `travel` uses `via: target`. `location.to` names a place, or `target`. Playing `to` without `teleport: true` is refused. `teleport: true` in `rules/verbs.yaml` is a warning.

`note` is stored on the save as `messages`. A note is shown to everyone named in `to`, wherever they are, and to anyone standing in `place`. It is hidden from a viewer when one of its `fact_ids` is one that viewer may not know. Place bodies and note text are shown as written. A hidden fact's sentence pasted into either one reaches every reader, and `spook check` does not warn. `narrate` is one-off text for the people it names, written on the round and left out of the save. `reveal` adds its fact to that actor's known facts and writes no memory line. A flag with `who: world` is stored on the save as `flags`.

`wield` fills the actor's one wielded slot. `unwield` empties it. `who` on an effect is the same set as on a condition.

## Check codes

`spook check` prints one line per problem: `file:line: message (CODE)` when it has a line, or `file: path: message (CODE)` when the location is a key path. A clean pack prints `no problems` and exits 0. `WARNING` is printed and does not by itself fail the command. Any other code exits 1. `--json` prints the same records as one array, and `hint` is the fix.

The message and the hint can change. The code stays.

| Code | Fix hint |
|---|---|
| `SCHEMA` | The diagnostic's `hint`. A bad shape, an unknown key, or an unknown id. |
| `NOT_IN_SPEC` | `add <file>` when a file the loader requires is missing. |
| `PACK_REV_MISMATCH` | `run spook migrate to store the current hash` |
| `RULES_REV` | `start a fresh save` |
| `HAND_EDIT` | `run spook migrate to keep the edit, or restore the file` |
| `TRIGGER_LEAK` | `address that output to self, or to here when the room can see the change` |
| `WARNING` | An unreachable place: `add an exit that reaches it`. `teleport: true` in `rules/verbs.yaml`: `teleport belongs on an author rule, not in verbs.yaml`. A trigger cycle: `stop one rule in the cycle from setting the next watched value`. |
