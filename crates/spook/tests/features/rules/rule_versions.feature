Feature: Rule versions

  Scenario: Mismatched save
    Given a save with rules_rev 6 and an engine at 7
    When spook run or spook verify opens it
    Then it refuses with RULES_REV and says to start a fresh save

  Scenario: A journal from before seat presence left the save is refused
    Given a journal written by rules_rev 6 with seat presence in the save
    When spook run or spook verify opens it
    Then it refuses with RULES_REV and says to start a fresh save

  Scenario: Divergence is named
    Given logged effects that differ from the re-run
    Then spook verify names the tick and the first differing effect
