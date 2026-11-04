Feature: A player knows where they are and what they can do

  Scenario: Arrival shows the place prose, names and exits
    Given rowan starts in docks_street
    When play joins the table on a free loopback address
    And play types "travel docks_shop"
    Then play shows "The shop faces the water. Tins sit in a row."
    And play shows "Docks Shop"
    And play shows "Aldric"
    And play shows "stairs (locked) — 2 min"
    And play shows "docks_street — 0 min"
    And play does not show "Pc rowan"
    And play does not show "Npc aldric"

  Scenario: help shows a verb that takes an actor with the actors in sight
    Given the docks have a greet verb that takes an actor, and the CLI holds rowan
    When I type "help"
    Then help shows "greet <actor>: aldric, pc_rowan" and not "greet"

  Scenario: help shows a cast verb only while that cast member is here
    Given Aldric has an ask verb and the CLI holds rowan in the shop
    When I type "help"
    Then I see the lines "look", "ask aldric" and "travel <exit>: docks_street, stairs"
    And help shows "ask aldric" and not "ask <actor>: aldric"
    And help shows "travel <exit>: docks_street, stairs" and not "Exits:"
    And play does not show the line "claim"
    And no round record was written
    When I type "travel docks_street"
    And I type "help"
    Then help shows "travel <exit>: docks_shop" and not "ask aldric"

  Scenario: help hides a verb whose if fails
    Given the shop has a pray verb if rowan holds lamp_oil, and the CLI holds rowan
    When I type "help"
    Then help shows "wait" and not "pray"
    When I type "take lamp_oil"
    And I type "help"
    Then help shows "pray" and not "claim"

  Scenario: help shows the verb of an item lying in sight
    Given lamp_oil has a sniff verb, and the CLI holds rowan
    When I type "help"
    Then help shows "sniff lamp_oil" and not "claim"

  Scenario: look reprints the place from the view
    Given the CLI is connected as a player holding rowan
    When I type "look"
    Then I see the place name, exits and who is here, built from server messages
    And world_time is 1080

  Scenario: A busy traveller sees where they are
    Given the stairs exit costs 10 minutes
    And pc_wren is seated and not travelling
    When rowan travels via stairs at 21:30
    Then the seated player sees "on the stairs until 21:40"

  Scenario: Unknown input points at help
    Given a docks table
    When play joins the table on a free loopback address
    And play types "dance"
    Then play's error shows "The table does not understand that. Type help."
    And world_time is 1080
