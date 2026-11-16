---
schema: cast
id: oden
control_type: npc
name: Oden
idle:
  eligible: true
rules:
  - id: call_from_the_shed
    when: habit
    if:
      - not:
          at:
            subject: corin
            place: shed
      - not:
          note_exists: note_oden_hint
    do:
      - note:
          id: note_oden_hint
          to:
            - corin
          place: shed
          fact_ids: []
          text: "Oden calls from the shed: \"Bring iron, or bring a shoulder. And don't go mocking that goblin. It bites.\""
---
Oden works in the shed and stays out of the yard. He wants the cart's wheel set before dark, and he will put his shoulder to it with anyone who comes to the shed with the nail.
