---
schema: cast
id: npc_briar
kind: npc
display_name: Briar
facts: []
tick:
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
---
Briar blocks the yard and does not step aside.
