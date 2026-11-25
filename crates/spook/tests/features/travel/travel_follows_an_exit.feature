Feature: Travel follows an exit

  Scenario Outline: A live travel with no exit stays put, and the street exit moves when named by <named by>
    Given a docks table with a seated player
    When the player travels to "mira_rooms"
    Then world_time is 1080
    And Rowan is in docks_shop
    When the player travels to "<words>"
    Then world_time is 1085
    And Rowan is in docks_street

    Examples:
      | named by         | words                   |
      | the place's id   | docks_street            |
      | the place's name | Docks Street            |
      | the path's id    | docks_shop.docks_street |

  Scenario: A word two exits answer to is refused with both path ids, and the path's name picks one
    Given a docks table with a seated player and a second path to the street, alley, named back door
    When the player travels to "Docks Street"
    Then the player is refused with "AMBIGUOUS_NAME: Docks Street: alley, docks_shop.docks_street"
    And Rowan is in docks_shop
    When the player travels to "back door"
    Then Rowan is in docks_street
