Feature: Play shows readable text

  Scenario: A docks session shows the place and who is present
    Given a docks table
    When play joins the table on a free loopback address
    Then play shows "You are in Docks shop."
    And play shows "Here: Aldric, You"
    And play shows no protocol JSON

  Scenario: A say is the speaker and the text
    Given a docks table
    When play joins the table on a free loopback address
    And play says "hello"
    Then play shows "You: hello"
    And play shows no protocol JSON

  Scenario: A second player is refused in a sentence
    Given a docks table with a seated player
    When another play joins that table
    Then the command exits 1
    And stderr shows "The table is full. [SESSION_FULL]"
    And stderr has no protocol JSON
    And stdout is empty

  Scenario: Raw play shows protocol frames
    Given a docks table
    When play joins the table with raw frames
    Then play shows protocol frames

  Scenario: The resume shows the door note and the debt clock as sentences
    Given a docks table
    When thirteen ticks pass
    And play joins the table on a free loopback address
    Then play shows the door note and the debt clock as sentences
    And play shows no protocol JSON

  Scenario: Raw play keeps the resume as the raw digest
    Given a docks table
    When thirteen ticks pass
    And play joins the table with raw frames
    Then play shows the raw resume digest
