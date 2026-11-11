Feature: A player knows where they are and what they can do

  Scenario: Arrival shows the place prose, names and exits
    Given rowan starts in docks_street
    When play joins the table on a free loopback address
    And play types "travel docks_shop"
    Then play shows "The shop faces the water. Tins sit in a row."
    And play shows "Docks Shop"
    And play shows "Aldric"
    And play shows "stairs (locked) — 2 ticks"
    And play shows "Docks Street — 0 ticks"
    And play does not show "Pc rowan"
    And play does not show "Npc aldric"

  Scenario: help shows a verb that takes an actor with the actors in sight
    Given the docks have a greet verb that takes an actor, and the CLI holds rowan
    When I type "help"
    Then help shows "greet <actor>: Aldric" and not "greet"

  Scenario: help shows a cast verb only while that cast member is here
    Given Aldric has an ask verb and the CLI holds rowan in the shop
    When I type "help"
    Then I see the lines "look", "ask Aldric" and "travel <exit>: Docks Street, stairs"
    And help shows "ask Aldric" and not "ask <actor>: Aldric"
    And help shows "travel <exit>: Docks Street, stairs" and not "Exits:"
    And play does not show the line "claim"
    And no round record was written
    When I type "travel docks_street"
    And I type "help"
    Then help shows "travel <exit>: Docks Shop" and not "ask Aldric"

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
    Then help shows "sniff Lamp oil" and not "claim"

  Scenario: look reprints the place from the view
    Given the CLI is connected as a player holding rowan
    When I type "look"
    Then I see the place name, exits and who is here, built from server messages
    And world_time is 1080

  Scenario: A busy traveller sees where they are
    Given the stairs exit costs 10 ticks
    And pc_wren is seated and not travelling
    When rowan travels via stairs at tick 1290
    Then the seated player sees "on the stairs until tick 1300"

  Scenario: Unknown input points at help
    Given a docks table
    When play joins the table on a free loopback address
    And play types "dance"
    Then play's error shows "The table does not understand that. Type help."
    And world_time is 1080

  Scenario: A player can read and type the names they see
    Given a copy of the sample-d20 pack
    When play joins the sample table
    Then play shows "Lark"
    And play does not show "pc_lark"
    And play shows "Mill Lane (locked) — 15 ticks"
    And play does not show "mill_lane"
    When play types "help"
    Then play shows "ask Briar"
    And play shows "travel <exit>: Mill Lane, Loft, Wheel Shed"
    When play types "travel Mill Lane"
    Then play's error shows "The lane is shut."
    And no round record was written
    When play types "ask briar"
    Then play shows "Briar steps aside."
    When play types "travel Wheel Shed"
    Then play shows "Wheel chock: 1"
    When play types "take Wheel chock"
    Then play shows "You picked up the Wheel chock."

  Scenario: Two things here with one name are refused, naming both ids
    Given a copy of the sample-d20 pack where Oden is also called Briar and stands in the Gate Yard
    When play joins the sample table
    And play types "ask Briar"
    Then play's error shows "More than one thing here is called Briar: npc_briar, npc_oden."
    And no round record was written
    When play types "say briar hello"
    Then play's error shows "More than one thing here is called briar: npc_briar, npc_oden."
