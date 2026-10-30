---
schema: cast
id: keeper
kind: npc
display_name: Keeper
facts: []
flags:
  - tended
idle:
  eligible: true
rules:
  - id: tend_the_yard
    when: habit
    do:
      - flag:
          who: self
          key: tended
          value: true
  # more-habits
---
The keeper watches the lane when the yard is empty.
