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
          who: pc_corin
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
            - pc_corin
          place: wheel_shed
          fact_ids: []
          text: Ward the wheel with the nail, back at the cart in the yard, or brace the cart here with him and keep the nail.
---
Oden works the wheel in the shed and stays out of the yard.
