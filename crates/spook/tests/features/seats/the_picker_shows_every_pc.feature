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
