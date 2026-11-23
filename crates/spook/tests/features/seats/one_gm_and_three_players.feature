Feature: One GM seat and up to three player seats

  Scenario: Three players and a GM share one docks table
    Given a docks table
    When three players claim and a GM connects
    Then players is 3
    And the present actors are the three docks PCs

  Scenario: A claim for a held PC changes nothing
    Given a docks table with a seated player
    When a second player connects
    Then the hello is refused with "SEAT_TAKEN"
    And save.yaml is unchanged since the seat
    And players is 1
    And the present actor is rowan

  Scenario: A seated GM writes nothing
    Given a docks table with a seated GM
    Then save.yaml is unchanged since the table opened
    And players is 0

  Scenario: A claim for an NPC is refused
    Given a docks table
    When a player claims mira
    Then the hello is refused with "SCHEMA"
    And save.yaml is unchanged since the table opened

  Scenario: A second GM on the game port is refused
    Given a docks table with a seated GM
    When a second GM connects on the game port
    Then the hello is refused with "TABLE_FULL"
    And save.yaml is unchanged since the table opened
    And players is 0

  Scenario: A one-player pack plays
    Given a one-player docks table
    When the player waits
    Then world_time is 1085
    And one round was played
    When a second player connects
    Then the hello is refused with "SEAT_TAKEN"
    And players is 1

  Scenario: One player drops and the other two stay
    Given a docks table with three seated players
    When the first player drops
    Then players is 2
    And the present actors are harbormouse and wren

  Scenario: A player perceives only the notes left to that PC
    Given a docks table where one PC has a private note
    When three players claim
    Then only that PC's perceive shows the private note

  Scenario: Saying a note's words does not leave the note to the others
    Given a docks table where one PC has a private note
    When three players claim
    And that PC says the note's words and another PC waits
    Then only that PC's perceive shows the private note

  Scenario: With no GM each player line commits its own round
    Given a docks table with two seated players
    When both players wait
    Then each wait committed its own round

  Scenario: A place say reaches only the players in that place
    Given a docks table with two PCs in the shop
    When the shop player speaks to the place
    Then only the shop players receive the speech

  Scenario: A command before a claim is refused
    Given a docks table
    When an unclaimed player waits
    Then the hello is refused with "CHOOSE_PC"
    When a player claims an empty id
    Then the hello is refused with "SCHEMA"
    And the hello reply does not contain "CHOOSE_PC"
