Feature: A pack can turn idle ticking off

  Scenario: An empty table past its interval writes no round while idle is off
    When idle is off and a past-due wake arrives

  Scenario: A long absence writes no catch-up while idle is off
    When idle is off and a long absence wakes

  Scenario: A seated line and spook tick still advance while idle is off
    When idle is off and a player waits and spook tick runs

  Scenario: The GM commit still advances while idle is off
    When idle is off and the GM commits a queued wait

  Scenario: A value other than on or off is refused at load
    Given a docks table
    When the copy is broken by "idle maybe"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/tick.yaml"
    And the report names "SCHEMA"
    And the report says "not on or off"
    And the report has a location for "rules/tick.yaml"
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: A zero round interval is refused at load
    Given a docks table
    When the copy is broken by "zero round interval"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/tick.yaml"
    And the report names "SCHEMA"
    And the report says "tick_every_real_seconds"
    And the report has a location for "rules/tick.yaml"
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: A zero idle minute interval is refused at load
    Given a docks table
    When the copy is broken by "zero idle minutes"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/tick.yaml"
    And the report names "SCHEMA"
    And the report says "idle_minutes_per_tick"
    And the report has a location for "rules/tick.yaml"
    And stdout is empty
    And the pack bytes are unchanged
