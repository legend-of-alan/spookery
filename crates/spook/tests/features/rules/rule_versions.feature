Feature: Rule versions

  Scenario: Mismatched save
    Given a save with rules_rev 4 and an engine at 5
    When spook run or spook verify opens it
    Then it refuses with RULES_REV and says to start a fresh save

  Scenario: Divergence is named
    Given logged effects that differ from the re-run
    Then spook verify names the tick and the first differing effect
