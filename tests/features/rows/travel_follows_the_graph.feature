Feature: Travel follows the graph

  Scenario: A live travel off the graph stays put and a street travel moves
    Given a docks table with a seated player
    When the player travels to "mira_rooms"
    Then world_time is 1085
    And Rowan is in docks_shop
    When the player travels to "docks_street"
    Then world_time is 1090
    And Rowan is in docks_street
    And the second tick records the move to the street

  Scenario: A street travel waits until the GM commits
    Given a docks table with a GM and a seated player
    When the player travels to "docks_street"
    Then the player line is queued
    And Rowan is in docks_shop
    And world_time is 1080
    And no tick file was written
    When the GM commits the round
    Then world_time is 1085
    And Rowan is in docks_street
    And the tick records the move to the street
