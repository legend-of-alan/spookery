@v0
Feature: Strip Markdown bodies
  Proof: v0_c11_strip_bodies
  Copy docks, delete every Markdown body, and tests 1, 2, 3, 5, 6, and 7 still pass.

  Scenario: Stripped Markdown bodies still hide the ledger from Aldric after one hundred idle ticks
    Given a temp copy of packs/oneshot-docks with every Markdown body deleted
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    And stripped Markdown bodies omit "Mira keeps the shop" and "Bram walks"
    When the pack is ticked 100 times
    Then every tick exits 0
    And Aldric perceive does not contain "The lamp-oil ledger is short."
    And Aldric memories do not contain "The lamp-oil ledger is short."
    And rumors do not contain "The lamp-oil ledger is short."

  Scenario: Stripped Markdown bodies still leave assassination_debt unchanged across twelve ticks
    Given a temp copy of packs/oneshot-docks with every Markdown body deleted
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    And stripped Markdown bodies omit "Mira keeps the shop" and "Bram walks"
    When the pack is ticked 12 times
    Then every tick exits 0
    And tick_index is 12
    And clock assassination_debt has value 2 and max 6
    And clock assassination_debt is paused
    And clock assassination_debt requires presence
    And clock assassination_debt owners are pc_rowan

  Scenario: Stripped Markdown bodies still leave state_rev unchanged when killed after apply_effect
    Given a temp copy of packs/oneshot-docks with every Markdown body deleted
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    And stripped Markdown bodies omit "Mira keeps the shop" and "Bram walks"
    And the shard state_rev, lamp_oil, and proposals are recorded
    When the pack is ticked with a crash after apply_effect
    Then the tick is killed before commit
    And state/.txn does not exist
    And restart state_rev is unchanged
    And restart lamp_oil is unchanged
    And restart proposals are unchanged
    And state/proposals does not exist
    And state/memories does not exist

  Scenario: Stripped Markdown bodies still keep lamp_oil within stock_rate
    Given a temp copy of packs/oneshot-docks with every Markdown body deleted
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    And stripped Markdown bodies omit "Mira keeps the shop" and "Bram walks"
    Then docks_shop lamp_oil stock_rate is 4
    And docks_shop lamp_oil is at least 0
    When 13 ticks record docks_shop lamp_oil
    Then every tick exits 0
    And each docks_shop lamp_oil change stays within stock_rate
    And each recorded docks_shop lamp_oil quantity is at least 0

  Scenario: Stripped Markdown bodies still leave Mira idle when a player is in docks_shop
    Given a temp copy of packs/oneshot-docks with every Markdown body deleted
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    And stripped Markdown bodies omit "Mira keeps the shop" and "Bram walks"
    And the player pc_rowan is present with mira in docks_shop and world_time 360
    When the shard runs one manual tick with no catch-up
    Then mira place is docks_shop
    And mira last_act_tick is 0
    And bram place is docks_shop
    And bram last_act_tick is 1

  Scenario: Stripped Markdown bodies still replay twenty ticks to a byte-equal now.yaml
    Given a stripped start copy and played copy of packs/oneshot-docks
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    And stripped Markdown bodies omit "Mira keeps the shop" and "Bram walks"
    When the stripped played pack is kernel-ticked 20 times and replayed onto the stripped start
    Then the stripped replay now.yaml bytes equal the played now.yaml
