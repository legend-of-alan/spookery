---
schema: cast
id: oden
kind: npc
display_name: Oden
idle:
  eligible: true
rules:
  - id: hold_the_cart
    when: habit
    if:
      - at:
          subject: corin
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
            - corin
          place: wheel_shed
          fact_ids: []
          text: Oden is ready when you are.
---
Oden works in the shed and stays out of the yard.
