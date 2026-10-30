Feature: A human GM runs a round

  Scenario: The GM hears a player, answers, and commits an NPC habit
    Given a docks table
    When play and a human GM share a free loopback port
    And the player says hello to the GM
    Then the GM shows "Pc rowan: hello"
    When the GM says "good evening"
    Then play shows "GM: good evening"
    When the GM queues Mira's restock and commits
    Then world time advanced one live tick
    And the round log records Mira's restock

  Scenario: Two players queue and one commit runs both
    Given a docks table
    When two players and a human GM share a free loopback port
    And each player queues a wait
    Then the GM shows "Pc rowan queued: wait"
    And the GM shows "Pc wren queued: wait"
    When the GM commits the queue
    Then the round log records Rowan and Wren waiting

  Scenario: Each commit's roll shows once
    Given a docks table
    When play and a human GM share a free loopback port
    And the GM queues Rowan's grit and commits
    And the GM queues Rowan's grit and commits
    Then the GM shows 2 rolls

  Scenario: The GM's output is text
    Given a docks table
    When play and a human GM share a free loopback port
    And the player says hello to the GM
    Then the GM shows "Pc rowan: hello"
    And the GM shows no protocol JSON

  Scenario: Raw frames are JSON lines
    Given a docks table
    When a human GM sits with raw frames on a free loopback port
    Then the GM shows protocol JSON

  Scenario: A second GM is told the table is full
    Given a docks table
    When a human GM sits on a free loopback port
    And another GM joins that seat
    Then the command exits 1
    And stderr shows "The table is full. [TABLE_FULL]"

  Scenario: A killed GM frees the seat for the next GM
    Given a docks table
    When a human GM sits on a free loopback port
    And the GM is killed
    And another GM sits in that seat
    Then the GM shows "Commands:"

  Scenario: An unknown move is refused in a sentence
    Given a docks table
    When a human GM sits on a free loopback port
    And the GM queues an unknown move
    Then the GM's stderr shows "That is not a move. [UNKNOWN_MOVE]"

  Scenario: A GM with nothing listening leaves the pack
    Given a docks table
    When the operator runs gm with nothing listening
    Then the command exits 1
    And the pack bytes are unchanged

  Scenario: spook gm exits when spook run stops
    Given a docks table
    When a human GM sits on a free loopback port
    And spook run stops
    Then the GM exits

  Scenario: spook gm dials an overridden agent door
    Given a docks table
    When spook run listens with an overridden agent door
    And a human GM dials that door
    Then the GM shows "Commands:"
