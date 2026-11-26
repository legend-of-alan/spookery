---
schema: cast
id: briar
control_type: npc
name: Briar
idle:
  eligible: true
rules:
  - id: watch_the_gate
    when: habit
    if:
      - not:
          note_exists: note_gate_watch
    do:
      - note:
          id: note_gate_watch
          to:
            - corin
          place: yard
          text: The yard stays watched.
  - id: strike
    when:
      verb: strike
      target: actor
    if:
      - wielding:
          subject: self
          id: cudgel
      - meter:
          of: target
          id: hp
          op: ">="
          value: 1
    check:
      total: 1d20 + self.might
      success_at: 40
      partial_at: 34
    success:
      - meter:
          of: target
          id: hp
          add: -4
      - narrate:
          to: self
          text: The cudgel lands hard. Briar staggers.
    partial:
      - meter:
          of: target
          id: hp
          add: -2
      - meter:
          of: self
          id: hp
          add: -2
      - narrate:
          to: self
          text: You trade blows. Briar grunts, and his staff clips your jaw.
    miss:
      - meter:
          of: self
          id: hp
          add: -3
      - narrate:
          to: self
          text: Briar's staff cracks your ribs.
  - id: yield_the_gate
    when:
      change:
        meter: hp
        crosses: "<=0"
    if:
      - flag:
          of: self
          id: warden
          is: true
    do:
      - flag:
          of: world
          id: passage
          value: true
      - narrate:
          to: here
          text: Briar drops his staff and steps back from the gate.
  - id: ask
    when:
      verb: ask
      target: actor
    do:
      - narrate:
          to: self
          text: "Briar says: \"Best me with a weapon, before the lamps.\""
---
Briar keeps the gate with his staff. He wants to be bested in a fair fight before the lamps are lit, and he swings back at anyone who swings at him.
