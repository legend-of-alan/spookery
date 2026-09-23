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
  needs:
    rest:
      min: 0
      max: 5
  work:
    needs:
      - key: rest
        delta: 1
    resources: []
  notes:
    - id: note_mira_door
      due_clock: shop_day
      due_value: 1
      to:
        - pc_rowan
      place: docks_shop
      fact_ids: []
      text: Back at dusk. Oil on the shelf.
---
Mira keeps the shop neat and the tin out of sight.
