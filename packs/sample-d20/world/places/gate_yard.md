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
      - all:
          - any:
              - flag:
                  of: world
                  key: passage
                  is: true
              - has_item:
                  of: self
                  id: latch
          - flag:
              of: world
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
