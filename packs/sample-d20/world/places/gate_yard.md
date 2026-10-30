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
      - all:
          - any:
              - flag:
                  who: world
                  key: passage
                  is: true
              - has_item:
                  who: self
                  id: latch
          - flag:
              who: world
              key: chocked
              is: true
    locked_text: The lane is shut.
  - to: loft
    name: loft
  - to: wheel_shed
    name: wheel_shed
facts:
  - id: fact_gate_open
    text: The gate stands open.
  - id: fact_cart
    text: A mill cart blocks the lane. Its wheel is still turning.
---
Wagon ruts cross the mud. A stick leans by the post. Briar stands in the ruts, and a mill cart sits across the lane.
