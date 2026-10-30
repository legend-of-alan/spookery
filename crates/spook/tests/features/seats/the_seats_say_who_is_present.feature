Feature: The host's seats say who is present

  Scenario: The humans condition sees who is seated
    Given a pack with a habit rule that requires humans == 0
    When "pc_rowan" sits and a round commits
    Then the habit does not fire
    When "pc_rowan" leaves and a round commits
    Then the habit fires

  Scenario: A requires_presence clock pauses on an empty table
    Given a docks table with "pc_rowan" seated
    When a round commits
    Then the requires_presence clock is running
    When "pc_rowan" leaves and a round commits
    Then the requires_presence clock is paused

  Scenario: Say routing reaches only seated players
    Given a docks table with "pc_rowan" seated and "pc_wren" not seated
    When the GM says a line to the table
    Then "pc_rowan" hears it and "pc_wren" does not

  Scenario: The GM status reply lists who is seated
    Given a docks table with "pc_rowan" and "pc_wren" seated
    When "pc_wren" leaves
    Then table_status shows humans 1 and actors_present "pc_rowan"

  Scenario: The journal records who was seated when the round ran
    Given a docks table with "pc_rowan" and "pc_wren" seated
    When "pc_wren" leaves
    And a round commits
    Then the round's presence_humans is 1 and actors_present is "pc_rowan"
    And spook verify passes
