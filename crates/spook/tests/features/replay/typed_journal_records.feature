Feature: Journal records are typed

  Scenario: Old journals still verify
    Given a docks table
    When the operator runs tick
    And a canon file gains a byte
    And the operator runs migrate
    And the operator runs tick
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: A field with the wrong type is named
    Given a docks table
    When the operator runs tick
    And the tick record's tick_index is a string
    And the operator runs verify
    Then the command exits 1
    And stderr names the tick record and the field tick_index

  Scenario: An unknown key is refused
    Given a docks table
    When the operator runs tick
    And the tick record gains an unknown key
    And the operator runs verify
    Then the command exits 1
    And stderr names that unknown key

  Scenario: A retired effect class is still illegal
    Given a docks table
    When the operator runs tick
    And one effect class is need
    And the operator runs verify
    Then the command exits 1
    And stderr names that record and says ILLEGAL_CLASS

  Scenario: Written records round-trip
    Given a docks table
    When the operator runs 3 ticks
    Then each journal record parses back to the bytes that were written

  Scenario: A quiet tick omits the standard tool trace
    Given a docks table
    When the operator runs 2 ticks
    Then the first tick lists cast:mira/leave_door_note and omits the standard steps
    And the second tick omits tool_trace
