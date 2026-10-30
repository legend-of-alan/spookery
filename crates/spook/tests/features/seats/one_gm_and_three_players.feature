Feature: One GM seat and up to three player seats

  Scenario: Three players and a GM sit one docks table
    Given a docks table
    When three players and a GM sit
    Then humans is 3
    And the present actors are the three docks PCs

  Scenario: A fourth player is refused when the pack defines four PCs
    Given a docks table whose pack defines four PCs
    When three players sit
    And a fourth player connects
    Then the hello is refused with "TABLE_FULL"
    And save.yaml is unchanged since the seat

  Scenario: A hello for a held PC changes nothing
    Given a docks table with a seated player
    When a second player connects
    Then the hello is refused with "SEAT_TAKEN"
    And save.yaml is unchanged since the seat
    And humans is 1
    And the present actor is pc_rowan

  Scenario: A seated GM writes nothing
    Given a docks table with a seated GM
    Then save.yaml is unchanged since the table opened
    And humans is 0

  Scenario: A second GM leaves the table unchanged
    Given a docks table with a seated GM
    When a second GM connects over HTTP
    Then the hello is refused with "TABLE_FULL"
    And save.yaml is unchanged since the table opened
    And humans is 0

  Scenario: A hello for an NPC is refused
    Given a docks table
    When a socket sends player:mira
    Then the hello is refused with "SCHEMA"
    And save.yaml is unchanged since the table opened

  Scenario: A GM hello on the game port is refused
    Given a docks table
    When a socket sends gm on the game port
    Then the hello is refused with "NOT_IN_SPEC"
    And save.yaml is unchanged since the table opened

  Scenario: A one-player pack sits and plays
    Given a one-player docks table
    When the player waits
    Then world_time is 1085
    And a round record was written
    When a second player connects
    Then the hello is refused with "SEAT_TAKEN"
    And humans is 1

  Scenario: One player drops and the other two stay
    Given a docks table with three seated players
    When the first player drops
    Then humans is 2
    And the present actors are pc_harbormouse and pc_wren

  Scenario: A player perceives only a fact that PC knows
    Given a docks table where one PC knows a hidden fact
    When three players sit
    Then only that PC's perceive shows the hidden fact

  Scenario: Saying a hidden fact does not teach it
    Given a docks table where one PC knows a hidden fact
    When three players sit
    And that PC says the hidden fact and another PC waits
    Then only that PC's perceive shows the hidden fact

  Scenario: With a GM two players commit in one round
    Given a docks table with a GM and two seated players
    When both players wait
    Then both lines are queued for the GM
    And table status lists both queued lines
    When the GM commits the round
    Then the round log records both waits

  Scenario: With no GM each player line commits its own round
    Given a docks table with two seated players
    When both players wait
    Then each wait committed its own round

  Scenario: A place say reaches only the players in that place
    Given a docks table with two PCs in the shop
    When the shop player speaks to the place
    Then only the shop players receive the say

  Scenario: Three play clients reach their first perceive
    Given a docks table
    When three play clients sit on a free port
    Then each play client shows its first perceive

  Scenario: An empty hello is refused with the free PCs
    Given a docks table
    When a socket sends an empty hello
    Then the hello is refused with "CHOOSE_PC" and the missing list is '["pc_harbormouse","pc_rowan","pc_wren"]'
    When a socket sends player:
    Then the hello is refused with "SCHEMA"
    And the hello reply does not contain "CHOOSE_PC"

  Scenario: An empty hello with no free PC lists none
    Given a one-player docks table
    When a socket sends an empty hello
    Then the hello is refused with "CHOOSE_PC" and the missing list is "[]"
    And the hello reply does not contain "SEAT_TAKEN"

  Scenario: spook play with one free PC seats it without a prompt
    Given a one-player docks table
    When play joins that one-player table with no --as
    Then play received the first perceive
