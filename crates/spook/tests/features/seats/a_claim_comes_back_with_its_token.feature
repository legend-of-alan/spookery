Feature: A claim comes back with its token

  Scenario: A player who drops takes rowan back and catches up
    Given a docks table with a seated GM
    And a player claimed rowan and got token t1
    When the connection drops without release and 100 server messages pass
    And the player reconnects with token t1
    Then it holds rowan again and receives the messages after its last seq
    And a GM intent for rowan is refused while it is held

  Scenario: A dropped claim keeps rowan from another player
    Given a docks table
    And a player claimed rowan and got token t1
    When the connection drops without release
    Then another player's claim on rowan is refused with "SEAT_TAKEN"
    And status shows players 1 and actors_present "pc_rowan"

  Scenario: A released claim's token takes nothing back
    Given a docks table
    And a player claimed rowan and got token t1
    When the player releases rowan
    And the player reconnects with token t1
    Then the reconnect is refused with "UNKNOWN_ID"
    And a GM intent for rowan is accepted

  Scenario: Catch-up keeps the newest 256 messages and says when older ones are gone
    Given a docks table with a seated GM
    And a player claimed rowan and got token t1
    When the connection drops without release and 300 server messages pass
    And the player reconnects with token t1
    Then it holds rowan again with a gap and the newest 256 messages

  Scenario: A player who quits play gives rowan back
    Given a docks table
    When play joins the table on a free loopback address
    And play's input ends
    Then play exits 0
    And a new player can claim rowan

  Scenario: Play takes rowan back after the table drops its connection
    Given a docks table
    When play joins the table on a free loopback address
    And play says a line over the message cap
    Then play shows the shop again as rowan
    And status shows players 1 and actors_present "pc_rowan"
