---
schema: cast
id: keeper
kind: npc
facts: []
tick:
  eligible: true
  flags:
    - tended
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
