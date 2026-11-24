Feature: Triggers

  Scenario: Arrival hangs on the place
    When nell travels via door into docks_shop
    Then nell has flag heard_bell
    When wren teleports into docks_shop by an author rule
    Then wren has flag heard_bell

  Scenario: Downed at 0 hp
    Given briar has 3 hp and a story rule on hp crosses <=0 sets flag downed
    When an attack removes 4 hp
    Then briar has flag downed

  Scenario: A change rule does not fire while its watch stays true
    Given briar has 1 hp and a story rule on hp crosses <=1 sets flag downed
    When another round passes
    Then briar does not have flag downed

  Scenario: An arrival with a flag sets a flag and turns the world's tide
    Given nell has flag found_key and arrives at mira_rooms
    Then nell has flag found_ledger and the world's tide is 1

  Scenario: Two rules that set each other's flags stop at depth 4
    Given two rules that set each other's watched flag
    Then spook check warns of a cycle, and a round stops at depth 4 and still commits

  Scenario: A private change narrated to the subject does not leak
    Given a rule on an arrival narrates to the subject
    When the operator runs spook check
    Then the check exits 0
    And stdout says "no problems"

  Scenario: Entering the bog rolls stamina
    When nell enters the bog
    Then that arrival rolled stamina and spook verify reproduces it
