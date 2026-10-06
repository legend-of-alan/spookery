---
schema: cast/v0
id: mira
kind: npc
facts:
  - id: fact_ledger_short
    text: The lamp-oil ledger is short.
    hidden: true
tick:
  eligible: true
  tier: named
  plot_relevant: false
  range:
    - docks_shop
    - docks_street
    - mira_rooms
  schedule:
    - start: 6
      end: 18
      place: docks_shop
      activity: work
    - start: 18
      end: 6
      place: mira_rooms
  stock_rate:
    docks_shop:
      lamp_oil: 4
  work:
    - key: rest
      write: base
      delta: 1
  notes:
    - id: note_mira_door
      due_clock: shop_day
      due_value: 1
      to:
        - pc_rowan
      place: docks_shop
      fact_ids: []
      text: Back at dusk. Oil on the shelf.
  agenda:
    - row: restock
      pause: subsistence_clocks
      when_stock: true
    - row: travel_schedule
      pause: travel
      schedule: away
    - row: work
      pause: subsistence_clocks
      schedule: here
      activity: work
    - row: rest
      schedule: here
      activity: rest
    - row: drift_relationship
      pause: relationship_drift
      when_drift: true
    - row: spread_rumor
      when_rumor: true
    - row: leave_note
      when_note: true
    - row: advance_clock
      pause: named_plot_clocks
      when_clock: true
---
Mira keeps the shop neat and the tin out of sight.
