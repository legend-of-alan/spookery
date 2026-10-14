---
schema: item
id: tonic
name: Bitter tonic
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
          who: self
          id: tonic
    cost:
      items:
        tonic: 1
    do:
      - meter:
          who: self
          key: hp
          add: 4
---

A green glass vial. It smells of pine and regret.
