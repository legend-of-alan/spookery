Feature: Travel follows the map

  Scenario: No exit, no travel
    Given docks_street has no exit to mira_rooms
    When rowan travels to mira_rooms from docks_street
    Then the intent is refused with NO_EXIT and rowan stays put

  Scenario Outline: A locked exit shows its text from either end and changes nothing
    Given rowan stands in <place>, the stairs exit needs brass_key, rowan has none, and travel walks <walk>
    When rowan travels via stairs
    Then rowan sees "The stair door is locked." and no state changes

    Examples:
      | place      | walk                  |
      | docks_shop | at once               |
      | docks_shop | inside a nested check |
      | mira_rooms | at once               |

  Scenario: Hidden exit
    Given the trapdoor is hidden and rowan does not carry found_trapdoor
    Then rowan's view does not list the trapdoor

  Scenario: Explicit teleport
    Given a place rule with location to cellar, teleport true
    Then the PC reaches cellar with no exit

  Scenario: With no GM, a lone traveller arrives on the travel line
    Given the stairs exit costs 10 ticks
    When rowan travels via stairs at tick 1290
    Then the world is at tick 1300 and rowan is in mira_rooms
    And rowan's line shows "mira_rooms"

  Scenario: Arrival is stamped at its exact tick and the actor acts next round
    Given rounds are 5 ticks while seated and the stairs exit costs 2 ticks
    When rowan travels via stairs in the round starting at tick 1290
    Then the round still ends at tick 1295
    And Rowan is in mira_rooms
    And a move from rowan is accepted from the round starting at tick 1295

  Scenario: Rounds never shorten
    Then a seated round advances world_time by 5 and an empty one by 60

  Scenario: Fast-forward when all seated PCs are busy plays through both arrivals
    Given rowan and nell are seated and both busy, with no GM
    Then the table plays rounds to tick 1295 then tick 1300 without waiting on the wall clock
    And Bram has flag tolled

  Scenario: GM mode waits for commit
    Given a GM is seated and both PCs are busy
    Then nothing advances until the GM commits

  Scenario: Fast-forward is capped
    Given busy_until is 200 rounds away and max_fastforward_rounds is 12
    Then 12 rounds run and the rest continues on the next action
    And spook verify passes

  Scenario: A player map lists the places rowan visited, across sittings
    Given rowan visited the shop and mira_rooms in an earlier session
    When play joins the table again
    Then play's map lists the shop and mira_rooms and not the trapdoor
