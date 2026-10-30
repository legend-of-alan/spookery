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
          who: pc_lark
          place: wheel_shed
      - not:
          note_exists: note_cart
      - flag:
          who: world
          key: chocked
          is: false
    do:
      - note:
          id: note_cart
          to:
            - pc_lark
          place: wheel_shed
          fact_ids: []
          text: Oden keeps the wheel turning. The cart stays until that wheel is chocked.
---
Oden keeps the lane mill and stays out of the yard.
