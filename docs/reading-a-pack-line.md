# Reading a pack line

A pack line reads as a clause. Each word keeps one part of speech and one role:

| Word | Part of speech | Role |
|---|---|---|
| `subject` | subject | the actor doing the verb, move or resist |
| verb | verb | the action |
| `target` | direct object | the thing the verb acts on, and nothing else |
| `of` | preposition | the owner whose value or state the line reads or changes, e.g. `meter: hp, of: target` reads as "hp of the target" |
| `to` | preposition | where something ends up: a time, a place, or an indirect object |
| `from` | preposition | where something starts: a time or a place |
| `at` | preposition | when or where: a time or a place |
| `via` | preposition | the way through |

**One role per preposition.** A preposition is used only for the role it fits, and a word never changes role from one line to the next. `for` and `by` are not used. The number a roll has to beat is `threshold`, never `target`, because `target` is only ever the thing the verb acts on.

The fields themselves, and what check refuses, are in the [Pack reference](pack-reference.md#rules). This page says how to read them.

## Lines from the real packs

### An arrival

`packs/sample-d20/story/ending.yaml`:

```yaml
  - id: lane_clear
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
          text: Oden and Nell are on the lane. That is the end.
```

Read it as: *when a PC (the subject) arrives at mill_lane, set the flag finished of the world to true, and narrate to self.* `arrive` names the verb and its place together. `subject` is the actor whose arrival the rule watches; omitted, it is `any`. `of: world` says whose flag it is. `to: self` is where the narration ends up.

### A strike

`packs/sample-d20/rules/verbs.yaml`:

```yaml
  - id: strike
    when:
      verb: strike
      target: actor
    roll:
      check: strike
      threshold: 38
      partial_at: 32
    success:
      - meter:
          of: target
          key: hp
          add: "-(self.might / 10)"
```

Read it as: *the subject strikes an actor (the target). Roll strike against 38. On a success, subtract might of self over 10 from hp of the target.* The subject is the player or NPC who played the move, written `self`. `target: actor` says what the verb acts on; `of: target` says that the hp belongs to it.

### Taking an item

```yaml
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
```

Read it as: *the subject takes an item (the target): one fewer of it here, one more of it on self.* `id: target` names which item; `of` names whose count changes.

### A walk

`packs/oneshot-docks/rules/verbs.yaml`:

```yaml
    do:
      - location:
          via: target
```

Read it as: *move the subject via the target exit.* A location effect may name `to` instead: the place the subject ends up.

### A time of day

`packs/oneshot-docks/cast/mira.md`:

```yaml
  - id: restock_lamp_oil
    when: habit
    if:
      - cycle:
          name: day
          from: 480
          to: 960
    do:
      - item:
          of: place:docks_shop
          id: lamp_oil
          add: 4
```

Read it as: *while the day is from tick 480 to tick 960, add 4 lamp_oil of place:docks_shop.* A cycle window is `from` inclusive and `to` exclusive: tick 480 is inside it and tick 960 is not. When `from` is above `to`, the window wraps past the cycle's end.

### A resist

From the `shrug` rule in the [Pack reference](pack-reference.md#rules):

```yaml
      - meter:
          of: self
          key: nerve
          add: -1
          resist:
            subject: target
            check: nerve
            threshold: 4
```

Read it as: *subtract 1 from nerve of self, unless the target resists: the target rolls nerve against 4.* In `resist`, `subject` is the actor who rolls; omitted, it is the move's `target`.

### A meter in a number

`{ meter: { of: self, key: nerve }, plus: 2 }` reads as *nerve of self, plus 2*. In an [expression](spec.md) the owner comes first instead: `self.nerve`, `target.hp`, `world.dawn`, `place:loft.light`.

## How a roll compares to its threshold

A roll's total is its check's dice plus the rolling actor's stat meter. The total is compared to `threshold` (`crates/engine/rules/roll.rs`):

- `total >= threshold` is a **success**.
- `partial_at <= total < threshold` is a **partial**, only when the roll sets `partial_at`.
- Any lower total is a **miss**. A roll without `partial_at` is a success or a miss.

`partial_at` is an absolute total, not an offset, so it sits below `threshold`. When another player's move opposes this one, the opposing total is the threshold and the roll has no partial band.

A resist roll uses the same comparison without a partial band. On a success the effect is shrugged off and its `resisted` list runs in its place; on a miss the effect lands.
