Feature: The host sits the GM seat

  Scenario: Host without a pack is a usage error
    When the operator runs ai-gm with no pack
    Then the command exits 2
    And stderr shows usage
    And stdout is empty
    And nothing was dialed

  Scenario: Host with an extra argument is a usage error
    Given a docks table
    When the operator runs ai-gm with an extra argument
    Then the command exits 2
    And stderr shows usage
    And stdout is empty
    And nothing was dialed

  Scenario: Host with nothing listening leaves the pack
    Given a docks table
    When the operator runs ai-gm with nothing listening
    Then the command exits 1
    And the pack bytes are unchanged

  Scenario: A missing model setting does not dial
    Given a docks table
    When the operator runs ai-gm without the model setting
    Then the command exits 1
    And the listener accepted no connection

  Scenario: The host seat submits a say and reports a refused call
    Given a docks table with a seated player
    When the host seat says good evening and calls an unknown tool
    Then the player saw a say from the host
    And the tape from is gm
    And the host call was refused

  Scenario: A scripted host follows the turn rules
    When the scripted host plays the turn checks

  Scenario: Play, host, and a stub share one speech turn and one travel turn
    When play and host and a stub run a speech turn and a travel turn

  Scenario: The host dials the given address
    Given a docks table
    When the host is pointed at a free loopback address
    Then the host dialed that address
