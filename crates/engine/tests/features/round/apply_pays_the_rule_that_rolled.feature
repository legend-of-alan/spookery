Feature: Apply pays the rule that rolled

  Scenario: Apply pays the cost of the rule that resolve rolled for
    Given a docks pack where the shop's "rummage" costs 1 rest and the shared "rummage" costs 2
    When Rowan walks to the street and rummages in the same round
    Then Rowan's rest is 2
    And the journal records "place:docks_shop/rummage" and not "verb:rummage"
