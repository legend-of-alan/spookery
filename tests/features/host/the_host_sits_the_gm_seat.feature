Feature: The host sits the GM seat

  Scenario: Host without a pack is not in spec
    When the operator runs host with no pack
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty

  Scenario: Host with an extra argument is not in spec
    Given a docks table
    When the operator runs host with an extra argument
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty

  Scenario: Host with nothing listening leaves the pack
    Given a docks table
    When the operator runs host with nothing listening
    Then the command exits 1
    And the pack bytes are unchanged

  Scenario: A missing model setting does not dial
    Given a docks table
    When the operator runs host without the model setting
    Then the command exits 1
    And the listener on 9737 accepted no connection

  Scenario: The host seat submits a say and reports a refused call
    Given a docks table with a seated player
    When the host seat says good evening and calls npc_act
    Then the player saw a say from the host
    And the tape from is gm
    And the host call was refused

  Scenario: A scripted host follows the turn rules
    When the scripted host plays the turn checks

  Scenario: Play, host, and a stub share one speech turn and one travel turn
    When play and host and a stub run a speech turn and a travel turn
