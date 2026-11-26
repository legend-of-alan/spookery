---
schema: cast
id: keeper
control_type: npc
name: Keeper
idle:
  eligible: true
rules:
  - id: tend_the_yard
    when: habit
    do:
      - flag:
          of: self
          id: tended
          value: true
  # more-habits
---
The keeper watches the lane when the yard is empty.
