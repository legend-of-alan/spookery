---
schema: place
id: docks_shop
display_name: Docks Shop
facts:
  - id: fact_oil_on_shelf
    text: Oil is on the shelf.
  - id: fact_ledger_short
    text: The lamp-oil ledger is short.
    hidden: true
exits:
  - to: docks_street
    name: docks_street
  - to: mira_rooms
    name: stairs
    cost:
      time: 2m
    if:
      - has_item:
          who: self
          id: brass_key
    locked_text: The stair door is locked.
---
The shop faces the water. Tins sit in a row.
