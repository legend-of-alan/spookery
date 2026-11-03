Feature: The listener bounds input

  Scenario: A message over the cap is refused and the seat stays usable
    Given a docks table with a seated player
    When the player sends a message over the cap
    Then the player is refused with "LINE_TOO_LONG"
    And the player can wait

  Scenario: A 1 MiB message closes the socket and the save is unchanged
    Given a docks table
    When a socket sends a 1 MiB message
    Then the socket was closed without a reply
    And save.yaml is unchanged since the table opened
    And a second player can sit and wait

  Scenario: An HTTP body over 1 MiB is refused and the seat stays free
    Given a docks table
    When an HTTP body over 1 MiB is posted
    Then the response status is 413
    And a new GM can sit

  Scenario: A binary message is refused and the seat stays usable
    Given a docks table with a seated player
    When the player sends a binary message
    Then the player is refused with "SCHEMA"
    And the player can wait

  Scenario: A socket that sends nothing is closed after the hello timeout
    Given a docks table
    When a socket connects and sends nothing
    Then the socket is closed before the hello
    And save.yaml is unchanged since the table opened

  Scenario: A flood is limited and another player still commits
    Given a docks table with two seated players
    When one player sends 200 lines as fast as it can
    Then the committed rounds stay within the rate allowance
    And the other player's line commits before the flood's burst is spent

  Scenario: A player who stops reading is dropped
    Given a docks table
    When one player stops reading and the GM says many lines
    Then that player is disconnected
    And the other player keeps receiving frames

  Scenario: The GM seat confirms it is free
    Given a docks table with a seated GM
    When the GM releases the seat
    Then the server confirms the seat is free
    And a second player can sit and wait

  Scenario: A closed GM session cannot call tools
    Given a docks table with a seated GM
    When the GM releases the seat and keeps the session
    Then the server confirms the seat is free
    When another GM sits
    And the closed session calls say
    Then that call is refused and nothing was written

  Scenario: A release with a key is refused and the GM keeps the seat
    Given a docks table with a seated GM
    When the GM calls release with a key
    Then that call is refused with "SCHEMA"
    And the GM can still say
