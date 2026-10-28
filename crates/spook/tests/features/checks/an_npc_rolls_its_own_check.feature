Feature: An NPC rolls its own check

  Scenario: A GM-queued NPC notice succeeds
    Given a docks table where Mira can notice
    When the GM queues Mira's sure notice
    And the GM commits the round
    Then Mira is braced
    And the GM sees Mira's sure notice
    And the player sees no check frame
    And the round log resolves the check without faces
    And replaying the round reproduces the save

  Scenario: A GM-queued NPC notice fails
    Given a docks table where Mira can notice
    When the GM queues Mira's hard notice
    And the GM commits the round
    Then Mira slipped
    And the GM sees Mira's failed notice
    And the player sees no check frame
    And the round log resolves the check without faces
    And replaying the round reproduces the save

  Scenario: A kernel habit rolls its NPC's check
    When a kernel habit rolls a check for Mira
    Then Mira slipped
    And the round log records Mira's kernel brace
    And the round log resolves the check without faces
    And replaying the round reproduces the save

  Scenario: An NPC supplies the opposing total
    Given a docks table where Mira can oppose a mark
    When the GM queues Mira's defense
    And the player marks Mira
    Then the player line is queued
    When the GM commits the round
    Then Mira guarded and the mark missed
    And the player sees only their own check
    And the GM sees both rolls
    And the round log resolves the check without faces
    And replaying the round reproduces the save

  Scenario: A player line for another PC is not queued
    Given a docks table with a seated player
    When the player names another PC
    Then world_time is 1080
    And no round file was written
