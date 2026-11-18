Feature: An NPC rolls its own check

  Scenario: A GM-queued NPC notice succeeds
    Given a docks table where Mira can notice
    When the GM queues Mira's sure notice
    And the GM commits the round
    Then Mira is braced
    And the GM sees Mira's sure notice
    And the player sees no check frame
    And spook verify passes

  Scenario: A GM-queued NPC notice fails
    Given a docks table where Mira can notice
    When the GM queues Mira's hard notice
    And the GM commits the round
    Then Mira slipped
    And the GM sees Mira's failed notice
    And the player sees no check frame
    And spook verify passes

  Scenario: An engine habit rolls its NPC's check
    When an engine habit rolls a check for Mira
    Then Mira slipped
    And spook verify passes

  Scenario: A dodge stored in let that reads the target's vigor defends the target
    Given a docks table where Mira's vigor contests a mark
    When the player marks Mira
    Then the player line is queued
    When the GM commits the round
    Then the mark missed Mira
    And the player sees only their own check
    And the GM sees one roll read from the attack and dodge dice lines
    And spook verify passes

  Scenario: A command line with a player prefix is refused
    Given a docks table with a seated player
    When the player sends a line with a player prefix
    Then world_time is 1080
    And no round was played
