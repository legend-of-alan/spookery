---
schema: cast
id: npc_oden
kind: npc
display_name: Oden
idle:
  eligible: true
rules:
  - id: hold_the_cart
    when: habit
    if:
      - at:
          of: pc_corin
          place: wheel_shed
      - not:
          note_exists: note_cart
      - flag:
          of: world
          key: chocked
          is: false
    do:
      - note:
          id: note_cart
          to:
            - pc_corin
          place: wheel_shed
          fact_ids: []
          text: Oden is ready when you are.
---
Oden works in the shed and stays out of the yard.
