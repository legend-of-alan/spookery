---
schema: place
id: gate_yard
display_name: Gate Yard
exits:
  - to: mill_lane
    name: mill_lane
    cost:
      ticks: 15
    if:
      - flag:
          of: world
          key: passage
          is: true
      - flag:
          of: world
          key: chocked
          is: true
    locked_text: The lane is shut.
  - to: loft
    name: loft
  - to: wheel_shed
    name: wheel_shed
  - to: docks_street
    name: docks_street
facts: []
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
          key: chocked
          value: true
      - narrate:
          to: self
          text: The wheel holds. The cart settles.
---
Wagon ruts cross the mud behind the street. A stick leans by the post. Briar stands in the ruts, and a mill cart sits across the lane.
