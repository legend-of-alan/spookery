---
schema: place
id: yard
name: Yard
exits:
  - to: lane
    id: gate
    cost:
      ticks: 15
    if:
      - flag:
          of: world
          key: passage
          is: true
      - not:
          flag:
            of: world
            key: lost
            is: true
    locked_text: Briar keeps the gate shut.
  - to: lane
    id: cart
    cost:
      ticks: 15
    if:
      - flag:
          of: world
          key: wheel_set
          is: true
      - not:
          flag:
            of: world
            key: lost
            is: true
    locked_text: The cart won't move.
  - to: loft
    id: loft
  - to: shed
    id: shed
facts:
  - id: fact_nell_waiting
    text: Nell is waiting on the lane for her flour, past the gate and the cart.
  - id: fact_two_ways
    text: Briar opens the gate to someone who bests him. The cart rolls once its wheel is warded with iron or braced.
---
Wagon ruts cross the mud. Briar leans on his staff at the gate, and a mill cart with a loose wheel sits across the lane. A ladder goes up to a loft, and a shed stands off the yard. The lamps get lit at dusk.
