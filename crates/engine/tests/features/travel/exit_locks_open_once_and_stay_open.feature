Feature: Exit locks open once and stay open

  Scenario: The door opens when the key holder arrives and stays open after the key is gone
    Given the stair door in the shop needs brass_key
    When Mira arrives at the shop holding brass_key
    Then after that round the stair door is open
    When Mira drops brass_key
    Then Mira can still take the stairs

  Scenario: Another actor walks through an opened door
    Given Mira opened the stair door
    When Aldric, who has no key, takes the stairs
    Then Aldric reaches Mira's rooms

  Scenario: A pack effect locks it again
    Given Mira opened the stair door
    When a rule runs a flag effect on exit:docks_shop.stairs setting open to false
    Then the stair door refuses Aldric with "The stair door is locked."
    And it opens again only when someone holding brass_key is in the shop

  Scenario: The journal shows the door opening and verify passes
    Given the stair door in the shop needs brass_key
    When Mira arrives at the shop holding brass_key
    Then the journal has a flag effect setting open on exit:docks_shop.stairs to true
    And spook verify replays the journal and passes
