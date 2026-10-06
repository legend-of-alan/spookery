---
schema: cast/v0
id: bram
kind: npc
facts: []
tick:
  eligible: true
  tier: named
  plot_relevant: false
  range:
    - docks_street
    - docks_shop
  agenda:
    - row: drift_relationship
      pause: relationship_drift
      when_drift: true
    - row: spread_rumor
      when_rumor: true
    - row: advance_clock
      pause: named_plot_clocks
      when_clock: true
---
Bram walks the night watch between the street and the shop.
