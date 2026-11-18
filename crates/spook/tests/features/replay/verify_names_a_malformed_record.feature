Feature: Verify names a malformed journal record

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
