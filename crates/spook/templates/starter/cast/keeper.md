---
schema: cast
id: keeper
kind: npc
facts: []
tick:
  eligible: true
  range:
    - yard
    - lane
  flags:
    - tended
  habits:
    - id: tend
      runs_while_empty: true
      rows:
        - tend_the_yard
    # more-habits
---
The keeper watches the lane when the yard is empty.
