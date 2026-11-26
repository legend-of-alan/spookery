---
schema: place
id: gate_yard
name: Gate Yard
rules:
  - id: chock
    when:
      verb: chock
    if:
      - has_item:
          subject: self
          id: chock
    cost:
      items:
        chock: 1
    do:
      - flag:
          of: world
          id: chocked
          value: true
      - narrate:
          to: self
          text: The wheel holds. The cart settles.
---
Wagon ruts cross the mud behind the street. A stick leans by the post. Briar stands in the ruts, and a mill cart sits across the lane.
