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
