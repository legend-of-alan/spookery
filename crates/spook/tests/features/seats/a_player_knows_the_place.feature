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

  Scenario: help lists the words from the hello and the exits from the view
    Given the CLI is connected as a player holding rowan
    When I type "help"
    Then I see the lines "wait", "travel" and "stairs (locked) — 2 min"
    And play does not show the line "claim"
    And world_time is 1080

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

  Scenario: A pack without display names fails check
    Given a docks table
    When the copy is broken by "missing display_name"
    And the operator runs spook check
    Then the check exits 1
    And the report says "display_name"
