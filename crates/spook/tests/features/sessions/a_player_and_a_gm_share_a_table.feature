Feature: A player and a GM share a table

  Scenario: A move waits for the GM and lands when the GM commits
    Given rowan is in the shop and a GM is seated
    When the player travels to "docks_street"
    Then the GM sees rowan's travel queued
    When the GM commits the round
    Then the player sees rowan arrive at docks_street
    And the GM sees rowan in docks_street

  Scenario: With no GM a move commits its own round
    Given a docks table with a seated player
    When the player travels to "docks_street"
    Then the player sees rowan arrive at docks_street

  Scenario: The player and the GM hear each other
    Given rowan is in the shop and a GM is seated
    When the player says "evening"
    Then the GM hears pc_rowan say "evening"
    When the GM says "good evening" to the table
    Then the player hears the GM say "good evening"

  Scenario: look, help and map are answered at once and take no round
    Given rowan is in the shop and a GM is seated
    When the player looks
    Then the player is shown "Docks Shop" with an exit to "docks_street"
    When the player sends "help"
    Then the player is told "look"
    When the player sends "map"
    Then the player is told "Docks Shop"
    And the GM sees nothing queued
    And no round record was written

  Scenario: A locked exit is refused and nothing is queued
    Given rowan is in the shop and a GM is seated
    When the player travels to "stairs"
    Then the table refuses the player with NO_EXIT
    And the GM sees nothing queued

  Scenario: An unknown word is refused
    Given rowan is in the shop and a GM is seated
    When the player sends "dance"
    Then the table refuses the player with SCHEMA
    And the GM sees nothing queued

  Scenario: A player who closes leaves the table
    Given rowan is in the shop and a GM is seated
    When the player closes the seat
    Then the GM sees no player seated

  Scenario: After the GM closes a move commits its own round
    Given rowan is in the shop and a GM is seated
    When the GM closes the seat
    And the player travels to "docks_street"
    Then the player sees rowan arrive at docks_street
