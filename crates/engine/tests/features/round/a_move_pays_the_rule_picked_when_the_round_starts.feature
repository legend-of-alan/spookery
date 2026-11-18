Feature: A move pays the rule picked when the round starts

  Scenario: A move pays the cost of the rule picked when the round starts, and its check reads the round as the move lands
    Given a docks pack where the shop's "rummage" costs 1 rest and rolls against the actor's rest, and the shared "rummage" costs 2
    And the walk to the street costs 1 rest
    When Rowan walks to the street and rummages in the same round
    Then Rowan's rest is 1
    And the GM sees Rowan's check against a bar of 1
    And the journal records "place:docks_shop/rummage" and not "verb:rummage"
