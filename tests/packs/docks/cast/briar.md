---
schema: cast
id: briar
control_type: npc
name: Briar
idle:
  eligible: true
rules:
  - id: ask
    when:
      verb: ask
      target: actor
    do:
      - flag:
          of: world
          key: passage
          value: true
      - narrate:
          to: self
          text: Briar steps aside.
---
Briar blocks the yard and does not step aside.
