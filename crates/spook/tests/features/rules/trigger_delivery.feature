Feature: Trigger delivery

  Scenario: Narrate reaches the people in the room
    When nell travels into the shop and the bell narrates
    Then the tick narrates "The bell rings." to "pc_nell"
    And the tick does not narrate "The bell rings." to "pc_wren"
    And "pc_nell" is told "The bell rings."
    And spook verify passes on this pack

  Scenario: A narrate that repeats a hidden fact fails audit
    Given a verb narrates the ledger fact to pc_rowan
    When that narrate runs
    And the operator runs audit
    Then the audit exits 1
    And stdout names "FAIL pc_rowan"

  Scenario: Reveal teaches the fact that uncovers an exit
    Given the street has a hidden trapdoor
    When pc_nell reveals fact_trapdoor
    Then pc_nell knows fact_trapdoor
    And pc_nell can see the trapdoor
    And spook verify passes on this pack

  Scenario: A world flag can be set and a later rule can read it
    Given a verb sets world flag tide_turned and a rule marks whoever is present
    When pc_nell sets the tide
    Then the world has flag tide_turned
    And pc_nell has flag saw_tide
    And spook verify passes on this pack

  Scenario: Change summaries stay with the viewers who can see the change
    When nell travels into the shop and the bell narrates
    Then the GM summary lists "actor_place pc_nell"
    And "pc_nell"'s summary lists "actor_flag pc_nell heard_bell"
    And "pc_wren"'s summary does not list "actor_flag pc_nell heard_bell"

  Scenario: Tick counters stay with the GM
    When a round runs
    Then the GM summary lists a tick counter change
    And "pc_nell"'s summary lists no tick counter change

  Scenario: A learned fact is told only to that person
    When pc_nell learns fact_trapdoor while pc_wren is in the same room
    Then the GM summary lists "actor_known_fact pc_nell"
    And "pc_nell"'s summary lists "actor_known_fact pc_nell"
    And "pc_wren"'s summary does not list "actor_known_fact pc_nell"

  Scenario: An item change is seen in that room only
    When an item is taken from docks_shop
    Then the GM summary lists "place_item docks_shop lamp_oil"
    And "pc_rowan"'s summary lists "place_item docks_shop lamp_oil"
    And "pc_wren"'s summary does not list "place_item docks_shop lamp_oil"

  Scenario: A clock change is seen by its owners only
    When a clock owned by pc_nell ticks
    Then the GM summary lists "clock_value nell_tide"
    And "pc_nell"'s summary lists "clock_value nell_tide"
    And "pc_wren"'s summary does not list "clock_value nell_tide"

  Scenario: Entering and leaving are seen from both rooms
    When pc_nell travels from docks_street into docks_shop
    Then the GM summary lists "actor_place pc_nell"
    And "pc_nell"'s summary lists "actor_place pc_nell"
    And "pc_wren"'s summary lists "actor_place pc_nell"
    And "pc_rowan"'s summary lists "actor_place pc_nell"
    And "pc_harbormouse"'s summary does not list "actor_place pc_nell"

  Scenario: A world flag is seen by everyone
    When a rule sets world flag tide_turned
    Then the GM summary lists "world_flag tide_turned"
    And every seated player's summary lists "world_flag tide_turned"

  Scenario: Another person's change is seen only in the same room
    When pc_wren's meter changes while pc_nell is elsewhere
    Then the GM summary lists "actor_meter pc_wren grit"
    And "pc_harbormouse"'s summary lists "actor_meter pc_wren grit"
    And "pc_nell"'s summary does not list "actor_meter pc_wren grit"

  Scenario: Another person's item change is seen only in the same room
    When pc_rowan takes an item while pc_nell is in the same room
    Then the GM summary lists "actor_item pc_rowan lamp_oil"
    And "pc_nell"'s summary lists "actor_item pc_rowan lamp_oil"
    And "pc_wren"'s summary does not list "actor_item pc_rowan lamp_oil"

  Scenario: A message is seen only by those allowed
    When a message hidden from pc_wren is written
    Then the GM summary lists "message note_private"
    And "pc_nell"'s summary lists "message note_private"
    And "pc_wren"'s summary does not list "message note_private"

  Scenario: An idle round does not linger for the next player
    When the empty table narrates and then Rowan narrates
    Then "pc_rowan" is told "You are back."
    And "pc_rowan" is not told "The shop is quiet."

  Scenario: Check rejects a reserved world actor and a reveal that cannot apply
    Given a cast id world and a reveal of a missing fact
    When the operator runs spook check
    Then the check exits 1
    And the report says "reserved"
    And the report says "not a fact"
    And the report says "reveals to the world"

  Scenario: Replay
    Then spook verify passes after ticks that fired triggers
