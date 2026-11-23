# Pack reference

Reference for people and agents writing a pack. The walkthrough is [Write your first pack](first-pack.md). What the engine promises is [Engine guarantees](engine-guarantees.md).

`spook check <pack>` is the authority for keys. An unknown key is rejected. `spook check --json <pack>` prints one array of `file`, `line`, `column`, `path`, `code`, `message`, and `hint`. The fences below are one pack, and that check accepts them. A rule that names `oil`, `knife`, or `nerve` uses the item and meter files on this page.

The first key in each file is `schema`, with no version suffix. In a Markdown file that key sits in the frontmatter.

A place, item, cast member or story may have `name`, the name a player sees. Without one, a player sees its id as written.

## Layout

| Path | What it holds |
|---|---|
| `pack.yaml` | Id, title, system, the start place and PC, and the GM's tone. Tag `pack`. |
| `rules/verbs.yaml` | Shared verb rules. Tag `verbs`. |
| `rules/meters.yaml` | The meters every actor carries. Tag `meters`. |
| `rules/modifiers.yaml` | Optional. Modifiers that move an actor's meter for a while. Tag `modifiers`. |
| `rules/budget.yaml` | Round budgets and the idle policy. Tag `budget`. |
| `world/places/<id>.md` | One place: facts, the meters and flags only it carries, and that place's rules. |
| `world/map.yaml` | Every path between places. Tag `map`. Required; `paths: []` when there are none. |
| `world/items/<id>.md` | One item: name, tags, prose, and that item's rules. |
| `world/meters.yaml` | Optional. The meters the world holds once. Tag `meters`. |
| `cast/<name>.md` | One NPC or PC template, the meters only it carries, and its rules. |
| `story/<id>.yaml` | Rules owned by a story, and the meters and flags only it carries. Tag `story`. |
| `state/save.yaml` | The live world. It is the save, not canon. |

A rule's owner is the file it lives in. The full id is `verb:<id>`, `place:<place>/<id>`, `item:<item>/<id>`, `cast:<actor>/<id>`, or `story:<story>/<id>`. `spook check --rules <pack>` prints those ids when the pack has no problems.

A verb uses the wielded item's rule for it, then the target item's rule, then the target actor's, then the rule on the current place, then `rules/verbs.yaml`. A cast member's verb rule is picked by naming them while they share the actor's place, and a bare verb never reaches it. A player's `help` lists a verb only where this order reaches a rule for it and that rule's `if` holds now. When no rule matches, the move is refused before the round with `NOT_IN_SPEC`, and the turn is not spent. A word is a verb only when some rule in the pack names it, so `use` is a verb only in a pack where some rule says `verb: use`. A habit runs only for the NPC whose cast file holds it.

`pack_rev` in the save is the hash of `world/`, `cast/`, and `rules/`. After those files change, `spook migrate <pack>` stores the new hash. What migrate promises is in `docs/engine-guarantees.md`. Migrate does not fix a shape error.

## Pack

`start.place` is the root of the exit graph. `checkpoint_every` is optional. It is the number of rounds between journal checkpoints. The default is 100. Zero is rejected. The field is not part of the canon hash.

`gm_notes` is optional text for the GM: the pack's tone, its narrative style, and any story goal. It is sent unchanged to every GM seat in `status`, and the AI GM puts it after its own instructions on every model call, so keep it to a few sentences (`spook check` warns past 1000 characters). Write only that, like the example below. Don't put how the GM uses its tools, the seat rules, turn handling, output format, or 'you are the GM' instructions here. Those belong to the AI GM itself. A line such as 'End the turn when every player has a move queued' does not belong here. A pack without `gm_notes` still gets a working GM. The field is not part of the canon hash.

```yaml file=pack.yaml
schema: pack
id: reference
title: Reference
system: reference
start:
  place: yard
gm_notes: >-
  A cold farmyard at dusk. Narrate in the second person, in short sentences.
```

## Places and exits

A place file is Markdown. The header between the `---` lines is the frontmatter. The prose after the closing `---` is the body a viewer can read.

Only places own facts. A fact has `id`, `text`, and optional `hidden`, which defaults to false. A player in the place sees every fact that is not hidden. A hidden fact is seen only by an actor whose `known_facts` lists it.

`world/map.yaml` lists every path between places under `paths:`. Each `path` is one place id, then `->` or `<->`, then another place id. `a -> b` is a way from `a` to `b`, and `a <-> b` is a way both ways. A path may have `id`, `cost`, `if`, `locked_text`, and `hidden` together with `revealed_by`, and on a `<->` line each one holds from both ends. When the two directions differ, write two one-way lines. A place's exits are the paths that leave it, in file order. `id` is the word a player types for the exit at each end; left out, it is the id of the place at the other end. A player reads an exit as the name of the place it goes to when its word is that place's id, or else as its word. Two exits at one place with the same word are rejected, so two paths from one place to another need their own ids. Both ends are place ids. A hidden exit needs `revealed_by`, and that value is a fact id from this pack. A place no path reaches from the start, counting hidden and locked exits, is a warning. A player's `map` shows each place's exits as they last saw them: at the end of each round, every actor notes the exits they can see where they stand. A hidden exit revealed while they are elsewhere shows on the map only after they go back. Where an exit goes, its ticks and whether it is locked are read when `map` prints.

`flags` lists the flags an effect may set on this place, and `meters` lists the meters only this place carries, in the shape of `rules/meters.yaml`. A rule names them with `of: place:<id>`, or `of: here` for the actor's own place. `world/places/loft.md` declares both.

`cost.ticks` is the game time the walk takes, read for the walker like a rule's `cost.ticks`. Omitted means 0. An exit with an `if` is a door, and the `if`, in the same conditions as a rule, is what it takes to open it. A door starts locked: travel through it shows `locked_text` and changes nothing.

Each end of a two-way door opens on its own. Its place and word are what `places.open_exits` lists and what `exit:<place>.<exit>` names. To start a door already open, list its exit id under that place in `places.open_exits` in the starting save. Meeting its `if` alone opens it only at the end of each round, after change rules, when any actor in its place meets that `if`, and it stays open for everyone, whoever holds the key after. The engine opens it with an `exit_open` effect it records in the journal.

A door carries one flag, `open`, which the engine owns: a rule names it with `of: exit:<place>.<exit>` and `key: open`, sets it to `false` to lock the door again, and may watch it like any flag. An exit with no `if` has no flag and is always open. The `bar` rule below locks the cellar door again.

A walk that takes ticks puts the traveller on the exit as soon as it starts. Their place is then `exit:<place>.<exit>`, in neither room, so nobody in either room sees them, and they arrive when the walk finishes. The busy time is the rule's ticks plus the exit's ticks. A walk of no ticks goes straight to the other side.

```yaml file=world/places/yard.md
---
schema: place
id: yard
name: Yard
facts:
  - id: fact_yard_open
    text: The yard gate stands open.
  - id: fact_hidden_stair
    text: Ivy hides a stair.
    hidden: true
rules:
  - id: bell
    when:
      verb: bell
    cost:
      ticks: 1440
    do: []
  - id: step_through
    when:
      verb: step_through
    do:
      - location:
          to: lane
          teleport: true
  - id: bar
    when:
      verb: bar
    do:
      - flag:
          of: exit:yard.cellar
          key: open
          value: false
---
Gravel underfoot.
```

```yaml file=world/map.yaml
schema: map
paths:
  - path: yard <-> lane
    cost:
      ticks: 10
  - path: yard -> loft
    id: stair
    hidden: true
    revealed_by: fact_hidden_stair
  - path: loft -> yard
  - path: yard <-> cellar
    if:
      - has_item:
          subject: self
          id: oil
          count: 1
    locked_text: The cellar door is locked.
```

```yaml file=world/places/lane.md
---
schema: place
id: lane
name: Lane
facts:
  - id: fact_lane_empty
    text: The lane is empty.
---
The lane runs past the yard.
```

```yaml file=world/places/loft.md
---
schema: place
id: loft
name: Loft
facts:
  - id: fact_loft_rope
    text: The loft smells of rope.
flags:
  - lamp_lit
meters:
  - id: light
    min: 0
    base_cap: 6
    max: 6
    base: 0
    hidden: false
---
Rope, and a view of the yard.
```

```yaml file=world/places/cellar.md
---
schema: place
id: cellar
name: Cellar
facts:
  - id: fact_cellar_dark
    text: The cellar is dark.
  - id: fact_ledger_short
    text: The oil ledger is one tin short.
    hidden: true
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
          of: self
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

`control_type` is `npc` or `pc_template`. A template does not seat a player. The save does. The file name is not the id: `cast/pc-ada.md` has id `ada`. A cast page has no `facts`: what an actor knows is `known_facts` on the save.

`flags` lists the flags an effect may set on this actor. A flag effect or condition whose `of` is fixed by the pack, as for meters, names a flag that entity's file lists, or check refuses it. World flags are not listed: any name but `busy` is one. `meters` lists the meters only this actor carries, in the shape of `rules/meters.yaml`; see [Meters](#meters). `idle.eligible` says whether the idle runner may pick this NPC. `when: habit` is a habit of that NPC. The runner takes the first habit whose `if` holds and whose `do` list expands.

A meter never pauses. To stop a rule from advancing a meter while no player is seated, give the rule `if: players >= 1`.

```yaml file=cast/pc-ada.md
---
schema: cast
id: ada
control_type: pc_template
name: Ada
flags: []
idle:
  eligible: false
meters:
  - id: debt
    min: 0
    base_cap: 6
    max: 6
    base: 0
rules:
  - id: debt_grows
    when: habit
    if:
      - players:
          op: ">="
          value: 1
    do:
      - meter:
          of: self
          key: debt
          add: 1
---
Ada keeps the yard.
```

```yaml file=cast/keeper.md
---
schema: cast
id: keeper
control_type: npc
name: Keeper
flags:
  - tended
idle:
  eligible: true
meters:
  - id: shop_day
    min: 0
    base_cap: 6
    max: 6
    base: 0
rules:
  - id: tend
    when: habit
    if:
      - all:
          - cycle:
              id: day
              from: 480
              to: 960
          - players:
              op: "=="
              value: 0
          - meter:
              of: self
              id: nerve
              op: ">="
              value: 1
      - any:
          - flag:
              of: self
              key: tended
              is: false
          - knows:
              subject: ada
              fact: fact_yard_open
      - not:
          note_exists: note_tended
      - has_item:
          subject: self
          id: oil
          count: 1
      - wielding:
          subject: self
          id: knife
      - at:
          subject: self
          place: yard
      - meter:
          of: self
          id: shop_day
          op: "<"
          value: 3
      - players:
          op: "<="
          value: 3
      - meter:
          of: ada
          id: nerve
          op: ">"
          value: "self.nerve + 1"
    do:
      - flag:
          of: self
          key: tended
          value: true
      - note:
          id: note_tended
          to_ids:
            - ada
          place_id: yard
          fact_ids: []
          text: The yard is swept.
---
Keeper sweeps the yard.
```

## Story

A story file is YAML. Check refuses an empty `id` or one another story file has. Its rules take the id `story:<id>/<local>`. Verb resolution does not read them. `flags` lists the flags an effect may set on the story, and `meters` the meters only it carries, in the shape of `rules/meters.yaml`. A rule names them with `of: story:<id>`.

```yaml file=story/samples.yaml
schema: story
id: samples
flags:
  - marked
meters:
  - id: dread
    min: 0
    base_cap: 6
    max: 6
    base: 0
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
          of: self
          key: spent
          value: true
      - flag:
          of: story:samples
          key: marked
          value: true
      - meter:
          of: story:samples
          key: dread
          add: 1
```

## Rules

### Reading a rule line

A pack line reads as a clause. Each word keeps one part of speech and one role:

| Word | Part of speech | Role |
|---|---|---|
| `subject` | subject | the actor the verb is about: who plays the move, or who has, is at, knows or wields in a condition |
| verb | verb | the action |
| `target` | direct object | the thing the verb acts on, and nothing else |
| `of` | preposition | the owner of the meter, flag or item count the line names, e.g. `meter: hp, of: target` reads as "hp of the target" |
| `to` | preposition | where something ends up: a time, a place, or an indirect object |
| `from` | preposition | where something starts: a time or a place |
| `at` | preposition | when or where: a time or a place |
| `via` | preposition | the way through |
| `by` | preposition | who makes a nested check's save and is told its outcome |

**One role per preposition.** A preposition is used only for the role it fits, and a word never changes role from one line to the next. `for` is not used, and `by` names only who makes a nested check. The number a check has to reach is `success_at`, never `target`, because `target` is only ever the thing the verb acts on. `grammar::Object` in Rust is any grammatical object a line names, including but not limited to a direct object, an indirect object or the object of a preposition.

The arrival in `tests/packs/docks/story/ending.yaml`:

```
    when:
      change:
        arrive: mill_lane
        subject: pc
    do:
      - flag:
          of: world
          key: finished
          value: true
      - narrate:
          to: self
          text: The cart rolls out onto the lane. That is the end.
```

reads as *when a PC (the subject) arrives at mill_lane, set the flag finished of the world to true, and narrate to self.* `arrive` names the verb and its place together. `subject` is the actor whose arrival the rule watches; omitted, it is `any`.

The strike in `tests/packs/docks/rules/verbs.yaml`:

```
    when:
      verb: strike
      target: actor
    check:
      total: 1d20 + self.might
      success_at: 38
      partial_at: 32
    success:
      - meter:
          of: target
          key: hp
          add: "-(self.might / 10)"
```

reads as *the subject strikes an actor (the target): roll 1d20 plus might of self, succeed at 38, and on a success subtract might of self over 10 from hp of the target.* The subject is whoever played the move, written `self`. `target: actor` says what the verb acts on; `of: target` says the hp belongs to it.

The take in the same file:

```
      - item:
          of: here
          id: target
          add: -1
      - item:
          of: self
          id: target
          add: 1
```

reads as *one fewer of the target item here, one more of it on self.* `id: target` names which item; `of` names whose count changes. `location: { via: target }` in the same file reads as *move the subject via the target exit*; a location may name `to` instead, the place the subject ends up.

Mira's restock in `tests/packs/docks/cast/mira.md`:

```
    if:
      - cycle:
          id: day
          from: 480
          to: 960
    do:
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: 4
```

reads as *while the day is from tick 480 to tick 960, add 4 lamp_oil of place:docks_shop.* A cycle window is `from` inclusive and `to` exclusive: tick 480 is inside it and tick 960 is not.

In the `shrug` rule below, `check: { by: target, total: 2d6 + target.nerve, success_at: 4 }` reads as *the target saves: it rolls 2d6 plus its nerve, and succeeds at 4.* Its `success` list runs when the save succeeds, and its `miss` list, the nerve loss of self and the `shrugged` flag, when it fails. `by` is the actor who makes the save; omitted, it is `self`. `self` inside it is still the rule's actor.

In the `give_oil` rule below, the nested check's `success_at: "self.nerve + 2"` reads as *nerve of self, plus 2*: in an [expression](spec.md) the owner comes first, then its meter.

A check's `total`, `success_at` and `partial_at` are each a number, and each may roll dice (`crates/engine/rules/roll.rs`):

- `total >= success_at` is a success.
- `partial_at <= total < success_at` is a partial, only when the check sets `partial_at`.
- Any lower total is a miss. A check without `partial_at` is a success or a miss.

The check rolls when the move lands, after the gate admits it, its start cost is paid and its `let` has rolled: `total`, then `success_at`, then `partial_at`, each writing its own dice line. A contested defense is a `let` value that reads the target's stats, used as `success_at`, not a nested check (see `lunge` below). A nested check is a save with its own outcome: it rolls when the effects reach it, on the rule's own dice, and may have a partial band.

### Rule fields

A rule has `id`, `when`, and `do`. It may have `if`, `cost`, `start`, `finish`, `cooldown`, `cancellable`, `let`, `check`, `success`, `partial`, and `miss`.

`when` is `habit`, a map with `verb` and `target`, or a map with `change`. `target` is `none`, `actor`, `place`, `exit`, or `item`. An omitted `target` means `none`. `say` is speech, so that id is not a rule. A built-in player command, such as `say` or `look`, cannot be a verb. `when` has one of `verb` or `change`, not both.

A change rule fires when its watch becomes true. It does not fire again while the watch stays true. What the engine does with that firing is in [Engine guarantees](engine-guarantees.md). `story/samples.yaml` has a meter watch. The other watches are:

| Watch | Fields |
|---|---|
| Arrive | `arrive` is a place id, or `here` on a place rule. `subject` is `any` (the default), `pc`, `npc`, or an actor id. |
| Leave | `leave` and `subject`, same as arrive. |
| Meter | `meter` is a meter id, on any entity that carries it. `crosses` is an operator and a number, such as `<=0`. |
| Flag | `flag` is the flag. `becomes` is `true` or `false`. |
| Item | `item` is an item id, with `gained: true` or `lost: true`. |
| Knows | `knows` is a fact id. The watch is that fact becoming known. |

A change rule has no `cost`, `start`, `finish`, `cooldown`, or `cancellable`. Check rejects them. It may have `if`, `check`, and the outcome lists. `if` is read after the change.

Matching change rules run in owner order (shared verbs, then place, item, cast, story), then by file path, then by position in the file. They run again on the changes they just wrote, until a pass matches nothing. In one pass a rule runs at most once for each subject. The subject is the actor whose value changed. A flag or meter of the world, a place, a story or an exit has no subject, so a rule watching one runs once per change, whoever is seated, and check refuses `self`, `here`, `location`, and `check` in it: name the actor, or `world`. A flag counts as the world's when a rule sets it with `of: world`; a world flag only the save sets is not checked, and a `self` there is refused when the rule runs. A key the world and an actor both carry fires the rule for each, and `TRIGGER_LEAK` reads it as an actor's change. Change rules run right after each action starts and right after each one finishes, so a rule sees every action's change and can change the world before the next action in the round. A walk fires `leave` at its start and `arrive` at its finish. Two changes to one key inside one action count as one. They run once more at the end of the round, after doors open. `max_trigger_depth` in `rules/budget.yaml` caps the passes each time they run. Omitted, it is 4. The next pass is recorded and does not run. Check warns when change rules set each other's watched values. A `narrate` or `note` on a change rule that can reach a viewer who cannot see that change is `TRIGGER_LEAK`.

`if` is a list. Each dash is one condition. `all` is and, `any` is or, and `not` negates one condition. Two conditions under one dash are rejected. `subject` on `has_item`, `at`, `knows`, and `wielding` is the subject of that verb: `has_item: { subject: self, id: key }` reads as *self has the key*. It is an actor: `self`, `target`, or an actor id; check refuses any other owner on that word's line. `of` on a meter, flag, or item is the owner whose value or count the line reads or changes: `self`, `target`, `here`, `world`, `place:<id>`, `story:<id>`, `exit:<place>.<exit>`, or an actor id. An exit holds only its flag `open`. An owner `target`, in a slot or a number such as `target.hp`, is the move's target, and check refuses it unless the rule targets an actor. On `has_item`, `wielding`, an item effect, `wield`, and a cost's `items`, `id: target` is the intent's target, and check refuses it unless the rule targets an item. A habit and a change rule target nothing, and a path's `if` and `cost` cannot say `target`. The habit on `cast/keeper.md` uses each condition. `op` is `<`, `<=`, `==`, `>=`, or `>`. `cycle` holds while the tick of the cycle `id`, world time modulo that cycle's length in `cycles`, is in `from..to`. When `from` is above `to` the window wraps past the cycle's end. `from` and `to` are ticks below the cycle's length and they differ; check compares them when they are integers, and a bound read from a meter that falls outside the cycle refuses the move `OUT_OF_RANGE`.

Every move but `say` is an action that starts and finishes. `cost` is what the start costs, and it may name `ticks`, `meters`, and `items`. `cost.ticks` is how long the action takes, read for the acting actor; omitted, it finishes in the round it starts. A count below 0 refuses the move `OUT_OF_RANGE`. An exit's ticks in a view and on `map` are read for the viewer, and show 0 when they cannot be read. A cost the actor cannot pay is refused when the move is played, and the world stays as it was. The start is checked again when the round runs it, since an earlier move may have changed the world.

`start.counts_as_act` says whether the start uses the actor's act this round. Omitted, it is `true`. An NPC that has used its act skips its habits that round, so a drink with `counts_as_act: false` leaves the act for one. `finish` may have `cost`, in `meters` and `items`, paid when the action finishes, and `counts_as_act`, which is `false` when omitted. A finish cost with `ticks` is rejected. Every effect lands at the finish, a timed walk's arrival and a timed teleport among them. `cooldown` is the ticks the actor stays busy after the finish. Omitted, it is 0. `cancellable: true` lets a player send the built-in command `cancel` while the action is under way: the action is dropped, its start cost is not refunded, and it never finishes. Omitted, it is `false`, and `cancel` is refused `INELIGIBLE`. A walk is never cancelled, whatever its rule says. The `chant` rule below takes five ticks, then three more to recover, and may be cancelled.

`check` has a `total` and a `success_at`. `partial_at` is an absolute total, strictly below `success_at` when both are integers, and it needs a `partial` list. Outcome lists need a `check`, and a `partial` or `miss` list needs a `success` list. `do` runs on every band. `success`, `partial`, and `miss` run on that band. A yield is `do: []` with no outcome lists.

A number is an integer or an [expression](spec.md). A number in a condition or a cost cannot roll dice: check refuses it on its line.

`let` is a map of ids to numbers that the rule rolls once and reads by id. It runs first, after the gate: once the move is admitted and its start cost is paid, each id rolls in the order written, then the check, then the effects. A move that takes time rolls its `let` at its finish, where its check rolls. A number reads an id as a bare word, `total: attack`, and a `let` number reads only the ids written above it. An id is letters, digits and `_`, not starting with a digit. It is not an owner word, such as `self` or `world`, and not `min`, `max`, `high` or `low`. Check refuses any other id at its line. An id written twice, or a second `let` in one rule, fails check at the second one's line with `'attack' is listed twice; keep one`. A roll used in more than one place goes in `let`. Dice written inline roll where they are written, so each list rolls its own inline dice, and a list that does not run rolls nothing. Each id that rolls writes one dice line with its id in front, `attack: 2d6 → 5, 2 = 7 + 1`. `high(dmg)` and `low(dmg)` read the higher and lower face of an id whose number is one dice group with no keep, such as `dmg: 2d4`. A contested defense is a `let` value that reads the target's stats, used as `success_at` and for `partial_at`, as in `lunge` below. A condition or a cost cannot read an id, since the gate reads them before `let` rolls: check refuses the read as dice. Check also refuses, at the rule's line, an id the rule's `let` does not hold, an id a `let` number reads at or above the line that holds it, and `high` or `low` of a number that is not one dice group with no keep. A number with a comma, such as `"-max(1, high(dmg) - 1)"`, needs quotes.

The flag key `busy` is rejected. An actor is busy from the start of a timed action until it finishes, and then through its cooldown. A move from a busy actor is refused `BUSY`, except `cancel`.

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
      ticks: 120
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
          of: here
          id: target
          add: -1
      - item:
          of: self
          id: target
          add: 1

  - id: press
    when:
      verb: press
    cost:
      ticks: 1
      meters:
        nerve: 1
      items:
        oil: 1
    check:
      total: 2d6 + self.nerve
      success_at: "6 + 2"
      partial_at: 4
    do:
      - narrate:
          to: ada
          text: You set your weight.
          fact_ids: []
    success:
      - flag:
          of: self
          key: pressed
          value: true
    partial:
      - flag:
          of: self
          key: pressed
          value: false
    miss:
      - narrate:
          to: self
          text: The press slips.
          fact_ids: []

  - id: meter_add
    when:
      verb: meter_add
    do:
      - meter:
          of: self
          key: nerve
          add: 1

  - id: meter_set
    when:
      verb: meter_set
      target: actor
    do:
      - meter:
          of: target
          key: nerve
          set: 3

  - id: chant
    when:
      verb: chant
    cost:
      ticks: 5
    start:
      counts_as_act: true
    finish:
      cost:
        meters:
          nerve: 1
      counts_as_act: false
    cooldown: 3
    cancellable: true
    do: []

  - id: shrug
    when:
      verb: shrug
      target: actor
    do:
      - check:
          by: target
          total: 2d6 + target.nerve
          success_at: 4
          success:
            - narrate:
                to: self
                text: They hold their nerve.
                fact_ids: []
          miss:
            - meter:
                of: self
                key: nerve
                add: -1
            - flag:
                of: self
                key: shrugged
                value: true

  - id: lunge
    when:
      verb: lunge
      target: actor
    let:
      attack: 2d6 + self.nerve
      dodge: 2d6 + target.nerve
      dmg: 2d4
    check:
      total: attack
      success_at: dodge
      partial_at: dodge - 2
    success:
      - meter:
          of: target
          key: nerve
          add: "-max(1, high(dmg) - 1)"
    partial:
      - meter:
          of: target
          key: nerve
          add: -low(dmg)

  - id: leave_note
    when:
      verb: leave_note
    do:
      - note:
          id: note_lane
          to_ids:
            - ada
          place_id: yard
          fact_ids: []
          text: A chalk mark points down the lane.

  - id: ring_bell
    when:
      verb: ring_bell
    do:
      - narrate:
          to: here
          text: A bell rings once.
          fact_ids: []

  - id: advance_day
    when:
      verb: advance_day
    do:
      - meter:
          of: keeper
          key: shop_day
          add: 1

  - id: dawn_nears
    when:
      verb: dawn_nears
    do:
      - meter:
          of: world
          key: dawn
          add: -1

  - id: remember
    when:
      verb: remember
    do:
      - memory:
          to: self
          fact_ids:
            - fact_yard_open
          text: The gate was open at dawn.

  - id: give_oil
    when:
      verb: give_oil
    do:
      - check:
          total: 2d6 + self.nerve
          success_at: "self.nerve + 2"
          miss:
            - item:
                of: here
                id: oil
                add: 1
                max: 4

  - id: show_stair
    when:
      verb: show_stair
    do:
      - reveal:
          to: self
          fact: fact_hidden_stair

  - id: scorch
    when:
      verb: scorch
      target: actor
    do:
      - modifier:
          to: target
          id: burn
          add: -2
          ticks: 10

  - id: arm
    when:
      verb: arm
    do:
      - wield:
          subject: self
          id: knife

  - id: disarm
    when:
      verb: disarm
    do:
      - unwield:
          subject: keeper
```

`travel` follows an exit of the current place. `via: target` is the exit the player named.

## Meters

`min` is the floor, `base_cap` is the trained cap, and `max` is as high as current can go. Current is base plus offset, and it is not stored. An optional `name` is what a player reads for the meter in a view and a change sentence. Without it, `spook play` shows the id with spaces for underscores and a capital first letter, so `assassination_debt` reads `Assassination debt`. A meter's entity is the one whose file declares it:

- `rules/meters.yaml` gives the meter to every actor.
- `meters:` in `cast/<name>.md` gives it to that one actor. The id is refused when `rules/meters.yaml` or another cast file has it too. `cast/keeper.md` declares `shop_day`.
- `meters:` in `world/places/<id>.md` gives it to that one place, and `meters:` in `story/<id>.yaml` to that one story. The id is refused when `rules/meters.yaml`, a cast, place or other story file has it too, and a story's also when `world/meters.yaml` has it. `world/places/loft.md` declares `light`, and `story/samples.yaml` declares `dread`.
- `world/meters.yaml` gives it to the world, held once in the save under `world.meters` and shared by every rule that names `of: world`. The world and an actor or place may each declare a meter with the same id, and `of` picks which one.

`base` is the base a new meter takes on its entity when `spook migrate` runs, and migrate drops a meter from an entity the pack no longer gives it. A save that lacks a meter the pack gives an entity, with no migrate to add it, is refused. So is a save that lists a meter on an entity the pack does not give it, with no migrate to drop it.

A meter effect, condition or read whose `of` is fixed by the pack names one entity: `world`, `place:<id>`, `story:<id>`, an actor id, or `self` on a cast member's own habit. Check refuses one that names a meter that entity does not carry. `target` and other entities known only at play time are not guessed.

Visibility: the GM sees every meter change. An actor's meter change is shown to that actor only, not to others in the same place. A world meter is hidden from players unless its row says `hidden: false`; then every player sees its changes and its current value in their view. A story or place meter is hidden the same way; with `hidden: false` every player sees a story meter's changes, and the players standing in a place see that place's, but neither is listed in a view. `hidden: true` keeps an actor's meter off that actor's own view. A flag change, on an actor, a place, a story or the world, is never shown to a player: use a meter with `hidden: false` or a fact for state players should see.

```yaml file=rules/meters.yaml
schema: meters
meters:
  - id: nerve
    name: Nerve
    min: 0
    base_cap: 100
    max: 110
    base: 1
```

```yaml file=world/meters.yaml
schema: meters
meters:
  - id: dawn
    min: 0
    base_cap: 6
    max: 6
    base: 6
    hidden: false
```

## Modifiers

A modifier changes one actor meter for a while, then expires. Each row has `id` and `meter`, and may have `max_stacks` and `hidden`. `meter` is a meter in `rules/meters.yaml` or a cast file, and check refuses any other.

A `modifier` effect names `to`, `id`, `add`, and `ticks`, such as `scorch` in `rules/verbs.yaml`. `ticks` is how long this application lasts, 1 or more, and check refuses an effect without it. `add` and `ticks` are integers or expressions, as on a meter; a `ticks` below 1 at play refuses the effect `OUT_OF_RANGE`. Any rule may apply any modifier, and each application keeps its own `add` as one stack. The meter's current value adds every stack of every modifier on it, so two modifiers on one meter both count. Nothing is written to the meter's offset.

`max_stacks` is optional. It is the most stacks one actor can hold of that modifier, from any source. The default is 1, so a modifier left without it does not stack. At `max_stacks`, an application stronger than the weakest stack, by absolute value, replaces it. A weaker or equal one changes no stack. Strength ignores sign, so use two modifiers for a buff and a debuff.

Below `max_stacks`, an application adds a stack. The modifier has one clock for all its stacks. It keeps the tick it expires at and `max_ticks`, the longest `ticks` applied so far. Every application, stronger, weaker or equal, adds its `ticks` to the ticks left, up to the longest burn so far, this one included. So the end becomes now plus the smaller of ticks left plus `ticks` and the larger of `max_ticks` and `ticks`, and `max_ticks` becomes that larger one. A 3 tick burn with 2 ticks left, hit by a 2 tick burn, has 3 ticks left. Hit by a 4 tick burn instead, it has 4. When the world reaches the end, the modifier expires and all its stacks go in that round. A modifier that should come from a place, such as fog, is applied to the actor by a change rule on that place.

The save keeps each actor's modifiers under `modifiers`, with the meter, the tick it ends at as `until`, `max_ticks`, and the list of stacks. `spook migrate` drops a modifier the pack no longer defines, or one whose row now names another meter. When a row lowers `max_stacks`, an actor holding more keeps them until the next application, which drops the weakest extras first. The GM sees each change as `modifier <actor> <id>`, and the meter's own change with it. A player sees that key for their own actor only, and not when the row says `hidden: true`.

```yaml file=rules/modifiers.yaml
schema: modifiers
modifiers:
  - id: burn
    meter: nerve
    max_stacks: 3
  - id: fog
    meter: nerve
    hidden: true
```

## Budget

`idle` is `on` or `off`. Omitted means `on`. Omitted counters use the engine defaults. Game time is counted in ticks, and players and the GM see plain ticks. `cycles` names each cycle a `cycle` condition reads, with its length in ticks; it may be left out. `idle_acts_per_window` caps an actor's idle acts in each window of `idle_act_window_ticks` ticks, and `live_acts_per_window` caps its live acts in each window of `live_act_window_ticks` ticks. `round_every_real_seconds` is real seconds between wakes, `idle_ticks_per_round` and `live_ticks_per_round` are the ticks one round writes, `idle_catchup_max_rounds` is the most rounds one catch-up runs, and `idle_max_ticks_per_real_hour` is the ticks one real hour of absence may write. A move past `max_intents_applied` or `max_npcs_acting` is refused `BUDGET`, and so is a rule that goes over `max_state_keys_mutated` and every move after it that round. The file below sets them.

```yaml file=rules/budget.yaml
schema: budget
idle: on
max_intents_applied: 12
max_npcs_acting: 8
max_effects_per_intent: 4
max_state_keys_mutated: 24
max_memory_writes: 8
idle_acts_per_window: 3
idle_act_window_ticks: 1440
live_acts_per_window: 6
live_act_window_ticks: 60
cycles:
  day: 1440
round_every_real_seconds: 300
idle_ticks_per_round: 60
live_ticks_per_round: 5
idle_catchup_max_rounds: 12
idle_max_ticks_per_real_hour: 480
max_fastforward_rounds: 12
max_trigger_depth: 4
```

## Effects

One effect under each dash. The rules in `rules/verbs.yaml` are the examples. `step_through` on the yard is the `location.to` example.

`check` under a dash is a nested check: a save with its own outcome. It has `total`, `success_at`, an optional `partial_at`, and `success`, `partial` and `miss` lists, any of which may be left out. Its outcome runs one list, which may hold further nested checks. `by` is the actor who makes the save and is told its outcome; omitted, it is `self`, and check refuses `by: target` unless the rule targets an actor. `self` inside it is still the rule's actor, and its dice are the rule's own, taken in order. `partial_at` and a `partial` list need each other, and a constant `partial_at` is below a constant `success_at`, as on a rule's check. A contested defense is not a nested check: it is a `let` value that reads the target's stats, used as `success_at`.

`location.via` names an exit from the actor's current place. `travel` uses `via: target`, which needs a rule that targets an exit. `location.to` names a place, or `target`, which needs a rule that targets a place. Playing `to` without `teleport: true` is refused. `teleport: true` in `rules/verbs.yaml` is a warning.

`note` names its readers as `to_ids` and its place as `place_id`. It is stored on the save under `globals.notes`, keyed by the note's `id`, with its `to_ids` and `fact_ids` each in order and listed once. A note is shown to everyone named in `to_ids`, wherever they are, and to anyone standing in `place_id`. It is hidden from a viewer when one of its `fact_ids` is one that viewer may not know. Place bodies and note text are shown as written. A hidden fact's sentence pasted into either one reaches every reader, and `spook check` does not warn. `narrate` is one-off text for the people its `to` names, written on the round and left out of the save. `to` is `self`, `here`, or an actor id. A world change rule may leave it out, and then it is every player present; check refuses a missing `to` anywhere else. `reveal` adds its fact to the actor `to` names and writes no memory line. `memory` writes a memory line to the actor `to` names and teaches them its facts. The save groups what each kind of entity holds by component: an actor's flags are under `actors.flags`, a place's items, meters and flags under `places.items`, `places.meters` and `places.flags` and its open doors under `places.open_exits`, a story's under `stories`, and the world's under `globals.meters`, `globals.flags` and `globals.notes`, each listed by the entity's id. The save lists no places or stories of its own: every place and story the pack defines exists.

`wield` fills the actor's one wielded slot. `unwield` empties it. `subject` on those effects, and on `has_item`, `at`, `knows`, and `wielding`, is the subject of that verb. `to` on `reveal`, `modifier`, and `memory` is who the fact, the modifier, or the memory line lands on: an actor, as `subject` is. `of` on a meter, flag, or item is the owner whose value or count the line reads or changes.

## Check codes

`spook check` prints one line per problem: `file:line: message (CODE)` when it has a line, or `file: path: message (CODE)` when the location is a key path. A clean pack prints `no problems` and exits 0. `WARNING` is printed and does not by itself fail the command. Any other code exits 1. `--json` prints the same records as one array, and `hint` is the fix when the message doesn't already say it.

The message and the hint can change. The code stays.

| Code | Fix hint |
|---|---|
| `SCHEMA` | The diagnostic's `message`, and its `hint` when it has one. A bad shape, an unknown key, or an unknown id. |
| `NOT_IN_SPEC` | `add <file>` when a file the loader requires is missing. |
| `PACK_REV_MISMATCH` | `run spook migrate to store the current hash` |
| `RULES_REV_MISMATCH` | `start a fresh save` |
| `HAND_EDIT` | `run spook migrate to keep the edit, or restore the file` |
| `TRIGGER_LEAK` | `address that output to self, or to here when the room can see the change` |
| `WARNING` | An unreachable place: `add a path in world/map.yaml that reaches it`. `teleport: true` in `rules/verbs.yaml`: `teleport belongs on an author rule, not in verbs.yaml`. A trigger cycle: `stop one rule in the cycle from setting the next watched value`. |
