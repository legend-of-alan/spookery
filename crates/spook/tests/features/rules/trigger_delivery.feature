Feature: Trigger delivery

  Scenario: Narrate reaches the people in the room
    When nell travels into the shop and the bell narrates
    Then "nell" is told "The bell rings."
    And "wren" is not told "The bell rings."
    And spook verify passes on this pack

  Scenario Outline: A flag on <carrier> uncovers an exit
    Given the street has an exit revealed by the found_trapdoor flag, which uncover sets on <carrier>
    When nell uncovers the trapdoor
    Then nell can see the trapdoor
    And wren <wren> see the trapdoor
    And spook verify passes on this pack

    Examples:
      | carrier | wren   |
      | self    | cannot |
      | world   | can    |

  Scenario: A rule watching a world flag runs when it turns
    Given a verb sets world flag tide_turned
    And a change rule watches tide_turned become true and adds 1 to meter tides of "world"
    And rowan and nell are seated
    When nell sets the tide
    Then the world has flag tide_turned
    And the world's tides is 1
    And spook verify passes on this pack

  Scenario: Change summaries stay with the viewers who can see the change
    When nell travels into the shop and the bell narrates
    Then the GM summary lists "actor_place nell"
    And "nell"'s summary lists "actor_place nell"
    And "nell"'s summary does not list "actor_flag nell heard_bell"
    And "wren"'s summary does not list "actor_flag nell heard_bell"

  Scenario: Round counters stay with the GM
    When a round runs
    Then the GM summary lists a round counter change
    And "nell"'s summary lists no round counter change

  Scenario: An item change is seen in that room only
    When an item is taken from docks_shop
    Then the GM summary lists "place_item docks_shop lamp_oil"
    And "rowan"'s summary lists "place_item docks_shop lamp_oil"
    And "wren"'s summary does not list "place_item docks_shop lamp_oil"

  Scenario: Entering and leaving are seen from both rooms
    When nell travels from docks_street into docks_shop
    Then the GM summary lists "actor_place nell"
    And "nell"'s summary lists "actor_place nell"
    And "wren"'s summary lists "actor_place nell"
    And "rowan"'s summary lists "actor_place nell"
    And "harbormouse"'s summary does not list "actor_place nell"

  Scenario: Another person's item change is seen only in the same room
    When rowan takes an item while nell is in the same room
    Then the GM summary lists "actor_item rowan lamp_oil"
    And "nell"'s summary lists "actor_item rowan lamp_oil"
    And "wren"'s summary does not list "actor_item rowan lamp_oil"

  Scenario: A message is seen only by those allowed
    When a message hidden from wren is written
    Then the GM summary lists "world_note note_private"
    And "nell"'s summary lists "world_note note_private"
    And "wren"'s summary does not list "world_note note_private"

  Scenario: An idle round does not linger for the next player
    When the empty table narrates and then Rowan narrates
    Then "rowan" is told "You are back."
    And "rowan" is not told "The shop is quiet."
