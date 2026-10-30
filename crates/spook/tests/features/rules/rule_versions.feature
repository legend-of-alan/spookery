Feature: Rule versions

  Scenario: A save from older rules is refused
    Given a save from older rules
    When spook run or spook verify opens it
    Then it refuses with RULES_REV_MISMATCH and says to start a fresh save

  Scenario: Divergence is named
    Given logged effects that differ from the re-run
    Then spook verify names the round and the first differing effect
