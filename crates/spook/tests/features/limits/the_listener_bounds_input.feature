Feature: The listener bounds input

  Scenario: A player sends a 1 MiB line with no newline
    Given a docks table
    When a player sends a 1 MiB line with no newline
    Then that player is refused with "LINE_TOO_LONG" and disconnected
    And a second player can sit and wait

  Scenario: An oversized hello is refused and the save is unchanged
    Given a docks table
    When a socket sends an oversized hello
    Then the hello is refused with "LINE_TOO_LONG"
    And now.yaml is unchanged since the table opened

  Scenario: A GM sends a JSON-RPC line over 1 MiB
    Given a docks table
    When a GM sends a JSON-RPC line over 1 MiB
    Then the GM session ends with "LINE_TOO_LONG"
    And a new GM can sit

  Scenario: Invalid UTF-8 in a player line is refused and the seat stays usable
    Given a docks table with a seated player
    When the player sends invalid UTF-8
    Then the player is refused with "SCHEMA"
    And the player can wait

  Scenario: A socket that sends nothing is closed after the hello timeout
    Given a docks table
    When a socket connects and sends nothing
    Then the socket is closed before the hello
    And now.yaml is unchanged since the table opened

  Scenario: A flood is limited and another player still commits
    Given a docks table with two seated players
    When one player sends 200 lines as fast as it can
    Then the committed ticks stay within the rate allowance
    And the other player's line commits within 2 seconds

  Scenario: A player who stops reading is dropped
    Given a docks table
    When one player stops reading and the GM says many lines
    Then that player is disconnected
    And the other player keeps receiving frames
