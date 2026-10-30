---
schema: cast
id: npc_briar
kind: npc
display_name: Briar
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
            - pc_lark
          place: gate_yard
          fact_ids: []
          text: The yard stays watched.
  - id: yield_the_lane
    when:
      change:
        meter: hp
        crosses: "<=0"
    if:
      - flag:
          who: self
          key: warden
          is: true
    do:
      - flag:
          who: world
          key: passage
          value: true
---
Briar blocks the yard and does not step aside.
