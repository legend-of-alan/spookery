---
schema: cast
id: mira
kind: npc
facts:
  - id: fact_ledger_short
    text: The lamp-oil ledger is short.
    hidden: true
tick:
  eligible: true
  range:
    - docks_shop
    - docks_street
    - mira_rooms
  stock_rate:
    docks_shop:
      lamp_oil: 4
rules:
  - id: leave_door_note
    when: habit
    if:
      - clock:
          id: shop_day
          op: ">="
          value: 1
      - not:
          note_exists: note_mira_door
    do:
      - note:
          id: note_mira_door
          to:
            - pc_rowan
          place: docks_shop
          fact_ids: []
          text: Back at dusk. Oil on the shelf.

  - id: restock_lamp_oil
    when: habit
    if:
      - hour:
          from: 8
          to: 16
    do:
      - stock:
          place: docks_shop
          sku: lamp_oil
          add: 4
---
Mira keeps the shop neat and the tin out of sight.
