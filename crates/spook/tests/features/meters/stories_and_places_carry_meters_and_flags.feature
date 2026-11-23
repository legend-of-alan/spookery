Feature: Stories and places carry meters and flags

  Scenario: A story meter belongs to its story
    Given story/ending.yaml declares a "dread" meter
    And a rule in story/ending.yaml adds 1 to meter dread of "story:ending"
    When the rule runs
    Then the GM sees dread change on the ending story
    And no player's view lists dread

  Scenario: A story flag is set and tested on its story
    Given a rule in story/siege.yaml sets flag walls_breached of "story:siege" to true
    When the rule runs
    Then a rule testing flag walls_breached of "story:siege" is true sees it as set
    And the GM sees the flag change on the siege story

  Scenario: A story meter shown by the pack reaches every player
    Given story/ending.yaml declares "dread" with hidden false
    And a rule in story/ending.yaml adds 1 to meter dread of "story:ending"
    And rowan in the shop and wren in the street are seated
    When the rule runs
    Then rowan's and wren's change lines show dread

  Scenario: A place flag is set and tested on its place
    Given the docks pack gains world/places/attic.md
    And a rule sets flag lamp_lit of "place:attic" to true
    When the rule runs
    Then a rule testing flag lamp_lit of "place:attic" is true sees it as set
    And the GM sees the flag change on the attic

  Scenario: A place meter shown by the pack reaches the players in the place
    Given world/places/attic.md declares "light" with hidden false and "draft" with no hidden key
    And rowan stands in the attic and Mira stands in the shop
    When a rule adds 1 to meters light and draft of "place:attic"
    Then rowan's change line shows light and not draft
    And Mira's change line shows neither
    And the GM sees both light and draft

  Scenario: A place flag is never shown to players
    Given the docks pack gains world/places/attic.md
    And rowan stands in the attic
    When a rule sets flag lamp_lit of "place:attic" to true
    Then rowan's change line does not show it

  Scenario: An item can be put in a place the pack added after its save was made
    Given the docks pack gains world/places/attic.md and is migrated
    When a rule adds 1 to item lamp_oil of "place:attic"
    Then the attic holds 1 lamp_oil
    And spook verify passes on this pack

  Scenario: A note can be left in a place the pack added after its save was made
    Given the docks pack gains world/places/attic.md and is migrated
    When a rule leaves a note in the attic
    Then rowan, standing in the attic, perceives the note
