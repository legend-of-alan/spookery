---
schema: cast
id: goblin
control_type: npc
name: Goblin
idle:
  eligible: false
meters:
  - id: nerve
    min: 0
    base_cap: 3
    max: 3
    base: 3
rules:
  - id: bite
    when: habit
    if:
      - at:
          subject: corin
          place: loft
      - meter:
          of: goblin
          id: nerve
          op: ">="
          value: 1
    check:
      total: 1d20 + self.might
      success_at: 12
    success:
      - meter:
          of: corin
          id: hp
          add: -2
      - narrate:
          to: corin
          text: The goblin darts out of the straw and bites your ankle.
    miss:
      - narrate:
          to: corin
          text: The goblin snaps at your ankle and misses.
  - id: banish
    when:
      verb: banish
      target: actor
    if:
      - meter:
          of: target
          id: nerve
          op: ">="
          value: 1
    check:
      total: 1d20 + self.grace
      success_at: 12
      partial_at: 7
    success:
      - meter:
          of: target
          id: nerve
          add: -1
      - narrate:
          to: self
          text: The goblin shrinks back into the straw.
    partial:
      - meter:
          of: self
          id: hp
          add: -2
      - narrate:
          to: self
          text: The goblin holds its ground and bites your hand.
    miss:
      - meter:
          of: self
          id: hp
          add: -3
      - modifier:
          to: self
          id: cursed
          add: -2
          ticks: 20
      - narrate:
          to: self
          text: The goblin spits a curse and claws your arm. Your hands shake.
  - id: flee
    when:
      change:
        meter: nerve
        crosses: "<=0"
    do:
      - item:
          of: self
          id: nail
          add: -1
      - item:
          of: here
          id: nail
          add: 1
      - narrate:
          to: here
          text: The goblin shrieks and burrows deep into the straw. The nail drops at your feet.
---
Something hunches in the straw: a goblin, or a knot of straw shaped like one. It wants the iron nail out of everyone's reach and bites whoever comes for it, but a firm word sends it burrowing. It can't stand being mocked, and an insult earns a bite.
