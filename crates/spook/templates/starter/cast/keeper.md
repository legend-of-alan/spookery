---
schema: cast
id: keeper
control_type: npc
display_name: Keeper
flags:
  - tended
idle:
  eligible: true
rules:
  - id: tend_the_yard
    when: habit
    do:
      - flag:
          of: self
          key: tended
          value: true
  # more-habits
---
The keeper watches the lane when the yard is empty.
