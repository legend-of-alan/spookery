---
schema: place
id: gate_yard
display_name: Gate Yard
exits:
  - to: mill_lane
    name: mill_lane
    cost:
      time: 15m
    if:
      - any:
          - flag:
              who: world
              key: passage
              is: true
          - has_item:
              who: self
              id: latch
    locked_text: Briar bars the lane.
  - to: loft
    name: loft
facts:
  - id: fact_gate_open
    text: The gate stands open.
    hidden: false
---
Wagon ruts cross the mud. A stick leans by the post.
