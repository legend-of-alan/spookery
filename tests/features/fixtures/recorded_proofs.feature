Feature: Recorded proofs stay stable

  Scenario: An empty tick log matches the recorded line
    When the empty tick log is checked

  Scenario: A flushed wait log matches the recorded line
    When the flushed wait log is checked

  Scenario: A budget tick log matches the recorded line
    When the budget tick log is checked

  Scenario: A catch-up tick log matches the recorded line
    When the catch-up tick log is checked

  Scenario: Schemas reject an unknown key
    When the schemas are checked for unknown keys

  Scenario: The now sample emits the recorded bytes
    When the now sample is emitted

  Scenario: The docks save round-trips
    When the docks save is checked

  Scenario: The scheduler sleeps and then catches up
    When the scheduler sleeps and catches up
