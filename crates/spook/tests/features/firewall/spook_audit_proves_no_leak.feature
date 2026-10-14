Feature: spook audit proves no hidden fact reached a player

  Scenario: A normal docks session passes audit
    Given a docks table
    When two players play, a GM commits a round, ticks pass, and one player drops
    And the pack bytes are remembered
    And the operator runs audit
    Then the audit exits 0
    And stdout has a pass line for "pc_harbormouse"
    And stdout has a pass line for "pc_rowan"
    And stdout has a pass line for "pc_wren"
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: A message that repeats the ledger fact is hidden and flagged
    Given a docks table
    When the door note text contains the ledger sentence
    And the evening tick runs on this copy
    Then Rowan does not perceive the ledger sentence
    When the operator runs spook check
    Then the check exits 1
    And the report names "cast/mira.md"
    And the report names "HIDDEN_FACT_IN_TEXT"
    And the report says "hidden fact"
    And the report has a location for "cast/mira.md"
    When the operator runs audit
    Then the audit exits 0
    And stdout has a pass line for "pc_rowan"
    And stderr is empty

  Scenario: A note id that is a hidden fact fails audit
    Given a docks table
    When a visible note's id is the ledger fact
    And the operator runs verify
    Then the command exits 0
    When the operator runs audit
    Then the audit exits 1
    And stdout names "FAIL pc_rowan"
    And stdout names "state_rev 0"
    And stdout names "fact_ledger_short"
    And stdout has a pass line for "pc_harbormouse"
    And stdout has a pass line for "pc_wren"

  Scenario: A GM say of the ledger fact is a note unless strict
    Given a docks table
    When two players sit and the GM says the ledger fact
    And the operator runs audit
    Then the audit exits 0
    And stdout has a note for speaker "gm" and recipient "pc_rowan"
    And stdout has a note for speaker "gm" and recipient "pc_harbormouse"
    And stdout says notes follow the journal
    When the operator runs audit --strict
    Then the audit exits 1
    And stdout has a note for speaker "gm" and recipient "pc_rowan"

  Scenario: A player say of the ledger fact is a note unless strict
    Given a docks table
    When two players sit and one says the ledger fact
    And the operator runs audit
    Then the audit exits 0
    And stdout has a note for speaker "player:pc_rowan" and recipient "pc_harbormouse"
    When the operator runs audit --strict
    Then the audit exits 1
    And stdout has a note for speaker "player:pc_rowan" and recipient "pc_harbormouse"
