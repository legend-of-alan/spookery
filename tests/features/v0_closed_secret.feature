@v0
Feature: Closed secret
  Proof: v0_t01_closed_secret
  The ledger sentence is absent from Aldric's perceive, memories, and rumors after 100 idle ticks. Mira can perceive fact_ledger_short because she knows it.

  Scenario: One hundred idle ticks leave Aldric ignorant of the ledger
    Given a temp copy of pack packs/oneshot-docks
    And cast mira.md contains "The lamp-oil ledger is short."
    And cast aldric.md does not contain "The lamp-oil ledger is short."
    When the pack is ticked 100 times
    Then every tick exits 0
    And Aldric perceive does not contain "The lamp-oil ledger is short."
    And Aldric memories do not contain "The lamp-oil ledger is short."
    And rumors do not contain "The lamp-oil ledger is short."
    And Mira perceive contains fact_ledger_short text "The lamp-oil ledger is short."
