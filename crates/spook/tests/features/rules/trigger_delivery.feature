Feature: Trigger delivery

  Scenario: Narrate reaches the people in the room
    When nell travels into the shop and the bell narrates
    Then the round narrates "The bell rings." to "nell"
    And the round does not narrate "The bell rings." to "wren"
    And "nell" is told "The bell rings."
    And spook verify passes on this pack

  Scenario: Reveal teaches the fact that uncovers an exit
    Given the street has a hidden trapdoor
    When nell reveals fact_trapdoor
    Then nell knows fact_trapdoor
    And nell can see the trapdoor
    And spook verify passes on this pack

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

  Scenario: A learned fact is told only to that person
    When nell learns fact_trapdoor while wren is in the same room
    Then the GM summary lists "actor_known_fact nell"
    And "nell"'s summary lists "actor_known_fact nell"
    And "wren"'s summary does not list "actor_known_fact nell"

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
    Then the GM summary lists "note note_private"
    And "nell"'s summary lists "note note_private"
    And "wren"'s summary does not list "note note_private"

  Scenario: An idle round does not linger for the next player
    When the empty table narrates and then Rowan narrates
    Then "rowan" is told "You are back."
    And "rowan" is not told "The shop is quiet."

  Scenario: Check rejects a reserved world actor and a reveal that cannot apply
    Given a cast id world and a reveal of a missing fact
    When the operator runs spook check
    Then the check exits 1
    And the report says "reserved"
    And the report says "not a fact"
    And the report says "reveals to the world"

  Scenario: Verify passes after rounds that fired change rules
    Then spook verify passes after rounds that fired triggers
