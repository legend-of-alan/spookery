Feature: Places own facts

  Scenario: A place fact with no hidden key is shown to a player there
    Given the docks pack where "fact_oil_on_shelf" is written without hidden
    When "pc_rowan" looks from "docks_shop"
    Then the perceive view for "pc_rowan" lists the fact "fact_oil_on_shelf"
    And the perceive view for "pc_rowan" does not list the fact "fact_ledger_short"
