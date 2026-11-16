---
schema: place
id: docks_shop
name: Docks Shop
facts:
  - id: fact_oil_on_shelf
    text: Oil is on the shelf.
  - id: fact_ledger_short
    text: The lamp-oil ledger is short.
    hidden: true
exits:
  - to: docks_street
    id: docks_street
  - to: mira_rooms
    id: stairs
    cost:
      ticks: 2
    if:
      - has_item:
          subject: self
          id: brass_key
    locked_text: The stair door is locked.
---
The shop faces the water. Tins sit in a row.
