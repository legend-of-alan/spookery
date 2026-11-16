Feature: Actions visibly matter

  Scenario: A strike keeps the target's hp off the player's line
    Given a docks table
    When harbormouse's might is raised and the seeded strike succeeds
    Then harbormouse's change line does not name briar's hp

  Scenario: Items gained or lost show up
    Given a docks table
    When harbormouse uses a tonic
    Then harbormouse sees one line saying the tonic was used up

  Scenario: An unseen gain does not turn a use into a move
    Given a docks table
    When harbormouse uses the tonic while someone out of sight gains one
    Then harbormouse sees one line saying the tonic was used up
    And that line does not mention the unseen gain

  Scenario: An item gained and given back in one action is no change
    Given a docks table
    When harbormouse plays a juggle that gains a tonic and gives it back
    Then the round names no change to harbormouse's tonic

  Scenario: Dropping an item is a move
    Given a docks table
    When harbormouse drops the tonic
    Then harbormouse sees one line saying they dropped the tonic

  Scenario: Taking an item is a move
    Given a docks table
    And a tonic is on the ground
    When harbormouse takes the tonic
    Then harbormouse sees one line saying they picked up the tonic

  Scenario: A move to another place shows the place's name
    Given a docks table
    When harbormouse climbs to the loft
    Then harbormouse sees one line naming the Loft as their place

  Scenario: A player sees their own meters
    Given a docks table
    And harbormouse has a hidden meter
    When harbormouse looks at their view
    Then harbormouse sees their own hp and might
    And harbormouse does not see a hidden meter

  Scenario: Changes the player may not know stay hidden
    Given a docks table
    And a round that changes a hidden fact
    Then the player's change line does not mention it
