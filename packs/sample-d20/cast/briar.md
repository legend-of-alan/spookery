---
schema: cast
id: npc_briar
kind: npc
facts: []
tick:
  eligible: true
  range:
    - gate_yard
    - mill_lane
    - loft
  habits:
    - id: watch_the_gate
      runs_while_empty: true
      unless_message: note_gate_watch
      rows:
        - watch_the_gate
---
Briar blocks the yard and does not step aside.
