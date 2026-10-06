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
  range:
    - docks_shop
    - docks_street
    - mira_rooms
  stock_rate:
    docks_shop:
      lamp_oil: 4
  habits:
    - id: door_note
      runs_while_empty: true
      clock: shop_day
      at_least: 1
      unless_message: note_mira_door
      rows:
        - leave_door_note
    - id: tend_shop
      runs_while_empty: true
      from_hour: 8
      to_hour: 16
      rows:
        - restock_lamp_oil
---
Mira keeps the shop neat and the tin out of sight.
