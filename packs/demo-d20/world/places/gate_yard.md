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
    locked_text: The lane is shut until the wheel is set and the way is clear.
  - to: loft
    name: loft
  - to: wheel_shed
    name: wheel_shed
facts:
  - id: fact_nell_waiting
    text: Nell is waiting on the lane, past the cart.
  - id: fact_cart
    text: The wheel has to be set before the lane will open.
---
Wagon ruts cross the mud. A stick leans by the post. Briar stands in the ruts, and a mill cart sits across the lane. A ladder goes up to a loft, and a wheel shed stands off the yard.
