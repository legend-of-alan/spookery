@v0
Feature: No self-win
  Proof: v0_t02_no_self_win
  assassination_debt stays unchanged across 12 empty ticks.

  Scenario: Twelve empty ticks leave assassination_debt unchanged
    Given a temp copy of pack packs/oneshot-docks
    When the pack is ticked 12 times
    Then every tick exits 0
    And tick_index is 12
    And clock assassination_debt has value 2 and max 6
    And clock assassination_debt is paused
    And clock assassination_debt requires presence
    And clock assassination_debt owners are pc_rowan
