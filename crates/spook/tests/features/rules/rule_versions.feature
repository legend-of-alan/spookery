Feature: Rule versions

  Scenario: Mismatched save
    Given a save with rules_rev 7 and an engine at 8
    When spook run or spook verify opens it
    Then it refuses with RULES_REV and says to start a fresh save

  Scenario: A save from before seat presence left the save is refused
    Given an old save from rules_rev 6 with seat presence in it
    When spook run or spook verify opens it
    Then it refuses with RULES_REV and says to start a fresh save

  Scenario: A journal presence record from older rules is refused
    Given a current save with a stray presence-0001.json in its journal
    When spook run or spook verify opens it
    Then it refuses with RULES_REV and says to start a fresh save

  Scenario: Divergence is named
    Given logged effects that differ from the re-run
    Then spook verify names the tick and the first differing effect
