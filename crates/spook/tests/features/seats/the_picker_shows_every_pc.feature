Feature: The character picker shows every PC

  Scenario: A new player's hello lists the open and the taken PCs
    Given a docks table with a seated player
    When an unclaimed player connects
    Then the hello lists "pc_harbormouse,pc_wren" as open and "pc_rowan" as taken

  Scenario: With one PC free play still shows the picker with every PC
    Given a docks table with two seated players
    When play joins that table as pc_wren
    Then play shows "You are in"
    And play shows the picker:
      """
      Choose a character:
      pc_harbormouse (in use)
      pc_rowan (in use)
      pc_wren
      """

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

  Scenario: play has no --as option
    When the operator runs the arguments "play" and "--as=pc_rowan"
    Then the command exits 2
    And stderr shows usage
