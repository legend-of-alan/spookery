Feature: Three static menus

  Scenario: pack-table lists the table tools
    Given a docks table
    When the operator lists the pack-table menu
    Then the menu is the table tools

  Scenario: pack-tick lists the tick tools
    Given a docks table
    When the operator lists the pack-tick menu
    Then the menu is the tick tools

  Scenario: pack-studio lists nothing
    Given a docks table
    When the operator lists the pack-studio menu
    Then the menu is empty

  Scenario: pack-gm is refused
    Given a docks table
    When the operator runs mcp pack-gm
    Then the command exits 1
    And stderr is NOT_IN_SPEC
