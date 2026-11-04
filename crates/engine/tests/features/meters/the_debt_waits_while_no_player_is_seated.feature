Feature: The debt waits while no player is seated

  Scenario: The debt rule waits while no player is seated
    Given pc_rowan's "assassination_debt" meter is 2
    And the rule that advances it has "if: players >= 1"
    When twelve empty rounds pass
    Then assassination_debt is still 2
