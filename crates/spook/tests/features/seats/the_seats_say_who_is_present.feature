Feature: The host's seats say who is present

  Scenario: The players condition sees who is seated
    Given a pack with a habit rule that requires players == 0
    When "rowan" sits and a round commits
    Then the habit does not fire
    When "rowan" leaves and a round commits
    Then the habit fires

  Scenario: Say routing reaches only seated players
    Given a docks table with "rowan" seated and "wren" not seated
    When the GM says a line to the table
    Then "rowan" hears it and "wren" does not

  Scenario: The GM status reply lists who is seated
    Given a docks table with "rowan" and "wren" seated
    When "wren" leaves
    Then status shows players 1 and actors_present "rowan"

  Scenario: The journal records who was seated when the round ran
    Given a docks table with "rowan" and "wren" seated
    When "wren" leaves
    And a round commits
    Then the round's presence_players is 1 and actors_present is "rowan"
    And spook verify passes
