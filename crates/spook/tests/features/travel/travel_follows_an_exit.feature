Feature: Travel follows an exit

  Scenario: A live travel with no exit stays put and a street exit moves
    Given a docks table with a seated player
    When the player travels to "mira_rooms"
    Then world_time is 1080
    And Rowan is in docks_shop
    When the player travels to "docks_street"
    Then world_time is 1085
    And Rowan is in docks_street
