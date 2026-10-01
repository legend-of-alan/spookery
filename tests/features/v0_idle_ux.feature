@v0
Feature: Idle UX
  Proof: v0_c09_idle_ux
  Host digest, player resume, and in-place perceive stay different views. Stock deltas appear in host status and do not appear in the player resume.

  Scenario: Stock deltas are in host status and not in the player resume
    Given a temp copy of pack packs/oneshot-docks
    When the pack is ticked 13 times
    Then every tick exits 0
    And tick_index is 13
    And world_time is 1860
    And last_tick_at is 2026-01-01T00:00:00Z
    And docks_shop lamp_oil is 8
    And clock assassination_debt value is 2
    And clock assassination_debt is paused
    When spook status is run
    Then spook status exits 0
    And spook status stdout is the host digest
    And status leaves the pack bytes unchanged
    And the player resume for pc_rowan is the idle digest
    And the player resume for pc_rowan does not contain lamp_oil
    And perceive as player:pc_rowan shows docks_shop stock lamp_oil 8
    And perceive as player:pc_rowan from mira_rooms shows no lamp_oil
