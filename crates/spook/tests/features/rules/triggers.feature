Feature: Triggers

  Scenario: Arrival hangs on the place
    When nell travels via door into docks_shop
    Then place:docks_shop/bell fires in the same tick
    And teleporting into docks_shop by an author rule also fires it

  Scenario: Downed at 0 hp
    Given briar has 3 hp and a story rule on hp crosses <=0 sets flag downed
    When an attack removes 4 hp
    Then briar has flag downed in the same tick

  Scenario: Edge, not level
    Given briar is already downed
    When another tick passes
    Then the rule does not fire again

  Scenario: Plot beat
    Given nell has flag found_key and arrives at mira_rooms
    Then nell knows fact_ledger and clock tide is running

  Scenario: Loops stop
    Given two rules that set each other's watched flag
    Then spook check warns of a cycle, and a tick stops at depth 4 and logs trigger_depth

  Scenario: A private change narrated to the room leaks
    Given a rule on a known fact narrates to the room
    When the operator runs spook check
    Then the check exits 1
    And the report names "TRIGGER_LEAK"
    And the report says "leak"

  Scenario: A private change narrated to the subject does not leak
    Given a rule on a known fact narrates to the subject
    When the operator runs spook check
    Then the check exits 0
    And stdout says "no problems"

  Scenario: Replay
    Then spook verify passes after ticks that fired triggers

  Scenario: Entering the bog rolls stamina
    When nell enters the bog
    Then that arrival rolled stamina and spook verify reproduces it
