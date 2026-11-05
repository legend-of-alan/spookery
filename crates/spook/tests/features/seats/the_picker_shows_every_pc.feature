Feature: The character picker shows every PC

  Scenario: With every PC taken play lists them all as in use
    Given a docks table with three seated players
    When the operator runs play on that table
    Then the command exits 0
    And stdout shows the picker:
      """
      Choose a character:
      pc_harbormouse (in use)
      pc_rowan (in use)
      pc_wren (in use)
      None is free.
      """

  Scenario: With every seat held play lists an unheld PC as free
    Given a docks table whose pack defines four PCs
    When three players sit
    Then a player's hello has only pc_lark open
    When another play picks pc_lark on that table
    Then the command exits 1
    And stderr shows "The table is full. [TABLE_FULL]"
    And stdout shows the picker:
      """
      Choose a character:
      pc_harbormouse (in use)
      pc_lark
      pc_rowan (in use)
      pc_wren (in use)
      """
