---
schema: item
id: tonic
display_name: Bitter tonic
tags:
  - consumable
  - healing
rules:
  - id: use
    when:
      verb: use
      target: item
    if:
      - has_item:
          of: self
          id: tonic
    cost:
      items:
        tonic: 1
    do:
      - meter:
          of: self
          key: hp
          add: 4
---

A green glass vial. It smells of pine and regret.
