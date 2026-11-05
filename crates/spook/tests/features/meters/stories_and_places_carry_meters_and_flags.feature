Feature: Stories and places carry meters and flags

  Scenario: A story meter belongs to its story
    Given story/ending.yaml declares a "dread" meter
    And a rule in story/ending.yaml adds 1 to who "story:ending" meter dread
    When the rule runs
    Then the GM sees dread change on the ending story
    And no player's view lists dread

  Scenario: A story flag is set and tested on its story
    Given story/siege.yaml declares the flag "walls_breached"
    And a rule in story/siege.yaml sets who "story:siege" flag walls_breached to true
    When the rule runs
    Then a rule testing who "story:siege" flag walls_breached is true sees it as set
    And the GM sees the flag change on the siege story

  Scenario: The pack check refuses an undeclared story flag
    Given story/siege.yaml declares no flags
    And a rule sets who "story:siege" flag walls_breached
    When the author runs spook check
    Then it refuses the rule, saying the siege story does not declare walls_breached

  Scenario: A story meter shown by the pack reaches every player
    Given story/ending.yaml declares "dread" with hidden false
    And a rule in story/ending.yaml adds 1 to who "story:ending" meter dread
    And pc_rowan in the shop and pc_wren in the street are seated
    When the rule runs
    Then pc_rowan's and pc_wren's change lines show dread

  Scenario Outline: A story meter id another file declares is refused
    Given <file> declares a "<meter>" meter
    And story/ending.yaml declares a "<meter>" meter
    When the author runs spook check
    Then it refuses the story's <meter> meter as "<why>"

    Examples:
      | file              | meter    | why                                                      |
      | world/meters.yaml | dawn     | is already in world/meters.yaml                          |
      | cast/mira.md      | shop_day | is already declared in another cast, place or story file |

  Scenario: The pack check refuses a story meter on the wrong story
    Given story/ending.yaml declares a "dread" meter
    And a rule adds 1 to who "story:prologue" meter dread
    When the author runs spook check
    Then it refuses the rule, saying only the ending story carries dread

  Scenario: A place flag is set and tested on its place
    Given world/places/loft.md declares the flag "lamp_lit"
    And a rule sets who "place:loft" flag lamp_lit to true
    When the rule runs
    Then a rule testing who "place:loft" flag lamp_lit is true sees it as set
    And the GM sees the flag change on the loft

  Scenario: The pack check refuses an undeclared place meter
    Given world/places/loft.md declares no meters
    And a rule adds 1 to who "place:loft" meter light
    When the author runs spook check
    Then it refuses the rule, saying the loft does not declare light

  Scenario: A place meter shown by the pack reaches the players in the place
    Given world/places/loft.md declares "light" with hidden false and "draft" with no hidden key
    And pc_rowan stands in the loft and Mira stands in the shop
    When a rule adds 1 to who "place:loft" meter light and meter draft
    Then pc_rowan's change line shows light and not draft
    And Mira's change line shows neither
    And the GM sees both light and draft

  Scenario: A place flag is never shown to players
    Given world/places/loft.md declares the flag "lamp_lit"
    And pc_rowan stands in the loft
    When a rule sets who "place:loft" flag lamp_lit to true
    Then pc_rowan's change line does not show it
