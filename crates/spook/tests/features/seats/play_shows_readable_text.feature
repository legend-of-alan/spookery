Feature: Play shows readable text

  Scenario: A docks session shows the place and who is present
    Given a docks table
    When play joins the table on a free loopback address
    Then play shows "You are in Docks Shop."
    And play shows "Here: Aldric, You"
    And play does not show "While you were away:"
    And play shows no protocol JSON

  Scenario: Speech is the speaker and the text
    Given a docks table
    When play joins the table on a free loopback address
    And play says "hello"
    Then play shows "You: hello"
    When play types "say Aldric hi"
    Then play shows "You: hi"
    When play types "tell aldric good evening"
    Then play shows "You: good evening"
    When play types "say place hi there"
    Then play shows "You: hi there"
    And play shows no protocol JSON

  Scenario: A second player is refused in a sentence
    Given a docks table with a seated player
    When another play picks rowan on that table
    Then the command exits 1
    And stderr shows "That character is already taken. [SEAT_TAKEN]"
    And stderr has no protocol JSON
    And stdout shows the picker:
      """
      Choose a character:
      Harbormouse
      Rowan (in use)
      Wren
      """

  Scenario: Raw play shows protocol frames
    Given a docks table
    When play joins the table with raw frames
    Then play shows protocol frames

  Scenario: The resume shows the door note as a sentence
    Given a docks table
    When thirteen rounds pass
    And play joins the table on a free loopback address
    Then play shows the door note as a sentence
    And play shows no protocol JSON

  Scenario: Raw play keeps the resume as the raw digest
    Given a docks table
    When thirteen rounds pass
    And play joins the table with raw frames
    Then play shows the raw resume digest

  Scenario: A round while seated shows no away heading
    Given a docks table
    When play joins the table on a free loopback address
    And play types "wait"
    Then the round shows no away heading

  Scenario: A note shows once per update after rejoin
    Given a docks table
    When thirteen rounds pass
    And play joins the table on a free loopback address
    And play types "wait"
    Then the round's update shows the door note only under Notes

  Scenario: Rejoining shows earlier speech under one heading
    Given a docks table
    When play joins the table on a free loopback address
    And play says "hello Briar"
    Then play shows "You: hello Briar"
    When play joins the table on a free loopback address
    Then play shows earlier speech under one heading

  Scenario: A strike keeps the target's health off the player's line
    Given a docks table where harbormouse's might is 40
    When play joins the table as "harbormouse"
    And play types "strike Briar"
    And play says "done"
    Then play shows "You: done"
    And play shows "Your roll succeeded."
    And play does not show "Health is now"

  Scenario: A used item shows as used up
    Given a docks table
    When play joins the table as "harbormouse"
    And play types "use Bitter tonic"
    Then play shows "Your Bitter tonic was used up. Your Health is now 12."

  Scenario: An unseen gain does not turn a use into a move
    Given a docks table where a sip costs harbormouse a tonic and gives Bram one
    When play joins the table as "harbormouse"
    And play types "sip"
    Then play shows "Your Bitter tonic was used up."
    And play does not show "gained"
    And play does not show "Bram"

  Scenario: An item gained and given back in one action is no change
    Given a docks table where a juggle gains harbormouse a tonic and gives it back
    When play joins the table as "harbormouse"
    And play types "juggle"
    And play says "done"
    Then play shows "You: done"
    And play does not show "Bitter tonic was"

  Scenario: Dropping an item is a move
    Given a docks table
    When play joins the table as "harbormouse"
    And play types "drop Bitter tonic"
    Then play shows "You dropped the Bitter tonic."

  Scenario: A move to another place shows the place's name
    Given a docks table
    When play joins the table as "harbormouse"
    And play types "travel Loft"
    Then play shows "Your place is now Loft."

  Scenario: Changes the player may not know stay hidden
    Given a docks table where a spill reveals a hidden fact to Briar and costs Briar 1 hp
    When play joins the table as "harbormouse"
    And play types "spill"
    And play says "done"
    Then play shows "You: done"
    And play does not show "Briar's hp"
    And play does not show "the sealed name"
