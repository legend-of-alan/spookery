Feature: The character picker shows every PC

  Scenario: With every PC taken play lists them all as in use
    Given a docks table with three seated players
    When the operator runs play on that table
    Then the command exits 0
    And stdout shows the picker:
      """
      Choose a character:
      Harbormouse (in use)
      Rowan (in use)
      Wren (in use)
      None is free.
      """

  Scenario: With every seat held play lists a free PC and picking it is refused as full
    Given a docks table whose pack defines four PCs
    When three players sit
    And another play picks lark on that table
    Then the command exits 1
    And stderr shows "The table is full. [TABLE_FULL]"
    And save.yaml is unchanged since the seat
    And stdout shows the picker:
      """
      Choose a character:
      Harbormouse (in use)
      Lark
      Rowan (in use)
      Wren (in use)
      """

  Scenario: A player picks a character by the name the picker shows
    Given a copy of the sample-d20 pack
    When play joins the sample table as "Lark"
    Then stdout shows the picker:
      """
      Choose a character:
      Lark
      Nell
      """
    And play shows "Gate Yard"
    And the present actor is lark
