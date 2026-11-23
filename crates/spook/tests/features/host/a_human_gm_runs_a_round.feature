Feature: A human GM runs a round

  Scenario: The GM hears a player, answers, and commits an NPC habit
    Given a docks table
    When play and a human GM share a free loopback port
    And the player says hello to the GM
    Then the GM shows "Rowan: hello"
    And the GM shows no protocol JSON
    When the GM says "good evening"
    Then play shows "GM: good evening"
    When the GM queues Mira's door note and commits
    Then world time advanced one live tick
    And the GM shows "world_note note_mira_door"

  Scenario: The GM speaks as an NPC where it stands
    Given a docks table
    When play and a human GM share a free loopback port
    And the GM types "say as aldric Mind the counter."
    Then play shows "Aldric: Mind the counter."
    And the tape line is said to the place "docks_shop"

  Scenario: Two players queue and one commit runs both
    Given a docks table
    When two players and a human GM share a free loopback port
    And each player queues a wait
    Then the GM shows "Rowan queued: wait"
    And the GM shows "Wren queued: wait"
    When the GM commits the queue
    Then the GM shows "actor_last_act_round rowan"
    And the GM shows "actor_last_act_round wren"

  Scenario: Each commit's roll shows once
    Given a docks table
    When play and a human GM share a free loopback port
    And the GM queues Wren's exert and commits
    And the GM queues Wren's exert and commits
    Then the GM shows 2 rolls

  Scenario: Raw frames are JSON lines
    Given a docks table
    When a human GM connects with raw frames on a free loopback port
    Then the GM shows protocol JSON

  Scenario: A second GM is told the table is full
    Given a docks table
    When a human GM connects on a free loopback port
    And another GM joins that seat
    Then the command exits 1
    And stderr shows "The table is full. [TABLE_FULL]"

  Scenario: A killed GM frees the seat for the next GM
    Given a docks table
    When a human GM connects on a free loopback port
    And the GM is killed
    And another GM connects to that seat
    Then the GM shows "Commands:"

  Scenario: An unknown move is refused in a sentence
    Given a docks table
    When a human GM connects on a free loopback port
    And the GM queues an unknown move
    Then the GM's stderr shows "That is not a move. [UNKNOWN_MOVE]"

  Scenario: A GM with nothing listening leaves the pack
    Given a docks table
    When the operator runs gm with nothing listening
    Then the command exits 1
    And the pack bytes are unchanged

  Scenario: spook gm exits when spook run stops
    Given a docks table
    When a human GM connects on a free loopback port
    And spook run stops
    Then the GM exits

  Scenario: The GM's status names who is seated and the queue
    Given a docks table
    When play and a human GM share a free loopback port
    And the GM types "status"
    Then the GM shows each of these blocks:
      """
      Seated: Rowan
      Queue: empty
      """

  Scenario: The GM perceives as an actor and sees no ids, habits or fact bodies
    Given a docks table
    When a human GM connects on a free loopback port
    And the GM types "perceive mira"
    Then the GM shows each of these blocks:
      """
      You are in Mira's Rooms.

      You know:
      The lamp-oil ledger is short.

      Meters:
      Hp: 8/16
      """
    And the GM does not show "The shop faces the water."
    And the GM does not show "leave_door_note"
    And the GM does not show "mira_rooms"
    And the GM does not show "fact_"
    When the GM types 'perceive {"as": "mira", "max_body_chars": 0}'
    Then the GM shows "You are in Mira's Rooms." twice
    And the GM does not show "fact_"

  Scenario: An unknown GM command points at help
    Given a docks table
    When a human GM connects on a free loopback port
    And the GM types "dance"
    Then the GM's stderr shows "The table does not understand that. Type help."

  Scenario: A GM command with wrong arguments is refused
    Given a docks table
    When a human GM connects on a free loopback port
    And the GM types "perceive a b"
    Then the GM's stderr shows "That line is not valid. [SCHEMA]"
