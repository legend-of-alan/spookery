@v0
Feature: Busy skip
  Proof: v0_t05_busy_skip
  A present player in docks_shop means Mira has no idle move. Another place may still act.

  Scenario: A present player in docks_shop means Mira has no idle move
    Given a temp copy of pack packs/oneshot-docks
    And the player pc_rowan is present with mira in docks_shop and world_time 360
    When the pack is ticked 1 times
    Then every tick exits 0
    And world_time advanced to 365
    And mira place is docks_shop
    And mira last_act_tick is 0
    And docks_shop lamp_oil is 4
    And bram place is docks_shop
    And bram last_act_tick is 1
