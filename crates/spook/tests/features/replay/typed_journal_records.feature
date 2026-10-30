Feature: Journal records are typed

  Scenario: Old journals still verify
    Given a docks table
    When the operator tries a round
    And a canon file gains a byte
    And the operator runs migrate
    And the operator tries a round
    And the operator runs verify
    Then the command exits 0
    And stdout says the save was verified
    And stderr is empty

  Scenario: A field with the wrong type is named
    Given a docks table
    When the operator tries a round
    And the round record's round_index is a string
    And the operator runs verify
    Then the command exits 1
    And stderr names the round record and the field round_index

  Scenario: An unknown key is refused
    Given a docks table
    When the operator tries a round
    And the round record gains an unknown key
    And the operator runs verify
    Then the command exits 1
    And stderr names that unknown key

  Scenario: A retired effect kind is unknown
    Given a docks table
    When the operator tries a round
    And one effect kind is need
    And the operator runs verify
    Then the command exits 1
    And stderr names that record and says UNKNOWN_EFFECT

  Scenario: Written records round-trip
    Given a docks table
    When the operator plays 3 rounds
    Then each journal record parses back to the bytes that were written

  Scenario: A quiet round omits the standard steps
    Given a docks table
    When the operator plays 2 rounds
    Then the first round lists only effect_applied and cast:mira/leave_door_note
    And the second round omits extra_steps
