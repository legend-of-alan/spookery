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
          text: Oden keeps the wheel turning. The cart stays until that wheel is warded, or until someone puts a shoulder to it.
---
Oden works the wheel in the shed and stays out of the yard.
