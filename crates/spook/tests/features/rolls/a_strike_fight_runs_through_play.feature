Feature: A strike fight runs through play

  Scenario: Harbormouse's strikes through play drop Briar to zero, and replay reproduces the save
    Given a docks table
    When play joins the table as "harbormouse"
    And the save before the fight is kept
    And play types "strike briar"
    And play types "strike briar"
    And play types "strike briar"
    And play types "strike briar"
    Then play shows 3 successful rolls
    And briar's hp is 0
    And replaying the round reproduces the save
