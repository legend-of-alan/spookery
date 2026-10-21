Feature: A hidden fact stays hidden

  Scenario: Stripped prose still hides the ledger and a repeated sentence is lint
    When prose is stripped and a hidden sentence is repeated

  Scenario: A message that repeats the ledger fact is hidden and flagged
    Given a docks table
    When the door note text contains the ledger sentence
    And the evening tick runs on this copy
    Then Rowan does not perceive the ledger sentence
    When the operator runs spook check
    Then the check exits 1
    And the report names "cast/mira.md"
    And the report names "HIDDEN_FACT_IN_TEXT"
    And the report says "hidden fact"
    And the report has a location for "cast/mira.md"
