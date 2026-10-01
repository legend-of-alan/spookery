@v0
Feature: Pack rev
  Proof: v0_pack_rev_mismatch_does_not_commit
  A pack_rev mismatch does not commit.

  Scenario: A pack_rev mismatch does not commit
    Given a temp copy of pack packs/oneshot-docks
    And state/now.yaml bytes are recorded
    When a newline is appended to cast/mira.md
    And the copied pack is ticked once
    Then the tick exits 1 with stderr PACK_REV_MISMATCH
    And state/now.yaml bytes are unchanged
    And no commit scratch was created
    And state/log does not exist
    And open_shard returns PACK_REV_MISMATCH
