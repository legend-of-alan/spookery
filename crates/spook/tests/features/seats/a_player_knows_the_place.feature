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
    And play does not show "rowan"
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
    And no round was played
    When I type "travel docks_street"
    And I type "help"
    Then help shows "travel <exit>: Docks Shop, Gate Yard" and not "ask Aldric"

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

  Scenario: A view shows what lies here, what the player carries and each meter by its name, and skips the empty sections
    Given a docks copy where rowan carries two tonics and wields the latch, the world shows dawn and the street's exits are hidden
    When play joins the table on a free loopback address
    Then play shows each of these blocks:
      """
      Items here:
      Lamp oil: 4

      You carry:
      Iron latch: 1 (in hand)
      Bitter tonic: 2

      Meters:
      Assassination debt: 2/6
      Health: 8/16

      The world:
      Dawn: 6/6
      """
    And play does not show "Notes:"
    And play does not show "docks_shop"
    When play types "travel Docks Street"
    Then the Docks Street view shows no exits and no Items here heading

  Scenario: A busy traveller sees where they are
    Given the stairs exit costs 10 ticks
    And wren is seated and not travelling
    When rowan travels via stairs at tick 1290
    Then the seated player sees "on the stairs until tick 1300"

  Scenario: Unknown input points at help
    Given a docks table
    When play joins the table on a free loopback address
    And play types "dance"
    Then play's error shows "The table does not understand that. Type help."
    And world_time is 1080

  Scenario: A player can read and type the names they see
    Given a docks table
    When play joins the table as "harbormouse"
    Then play shows "Harbormouse"
    And play does not show "harbormouse"
    And play shows "Mill Lane (locked) — 15 ticks"
    And play does not show "mill_lane"
    When play types "help"
    Then play shows "ask Briar"
    And play shows "travel <exit>: Mill Lane, Loft, Wheel Shed, Docks Street"
    When play types "travel Mill Lane"
    Then play's error shows "The lane is shut."
    And no round was played
    When play types "ask briar"
    Then play shows "Briar steps aside."
    When play types "travel Wheel Shed"
    Then play shows "Wheel chock: 1"
    When play types "take Wheel chock"
    Then play shows "You picked up the Wheel chock."

  Scenario: A cast member, place or item the pack gives no name shows its id
    Given a docks table where the shop, Aldric and the lamp oil have no name
    When play joins the table on a free loopback address
    Then play shows "You are in docks_shop."
    And play shows "Here: aldric, You"
    And play shows "lamp_oil: 4"

  Scenario: Two things here with one name are refused, naming both ids
    Given a docks table where Bram is called "Briar" and stands in the Gate Yard
    When play joins the table as "harbormouse"
    And play types "ask Briar"
    Then play's error shows "More than one thing here is called Briar: bram, briar."
    And no round was played
    When play types "say briar hello"
    Then play's error shows "More than one thing here is called briar: bram, briar."

  Scenario: Speech reaches a name with a space in it, and half the name stays at the table
    Given a docks table where Bram is called "Old Vell" and stands in the Gate Yard
    When play joins the table as "harbormouse"
    And play types "say Old Vell hi"
    Then play shows "You: hi"
    When play types "say Old hi"
    Then play shows "You: Old hi"
