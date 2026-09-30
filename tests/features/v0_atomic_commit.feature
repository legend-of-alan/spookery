@v0
Feature: Atomic commit
  Proof: v0_t06_atomic_commit
  Kill after apply_effect and before tick_commit. Restart leaves state_rev unchanged.

  Scenario: Kill after apply_effect leaves state_rev unchanged
    Given a temp copy of pack packs/oneshot-docks
    And the shard state_rev, lamp_oil, and proposals are recorded
    When the pack is ticked with a crash after apply_effect
    Then the tick is killed before commit
    And state/.txn does not exist
    And restart state_rev is unchanged
    And restart lamp_oil is unchanged
    And restart proposals are unchanged
    And state/proposals does not exist
    And state/memories does not exist
