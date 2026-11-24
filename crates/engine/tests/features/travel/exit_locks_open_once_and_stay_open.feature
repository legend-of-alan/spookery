Feature: Exit locks open once and stay open

  Scenario: The door opens when the key holder arrives and stays open after the key is gone
    Given the stair door in the shop needs brass_key
    When Mira arrives at the shop holding brass_key
    Then after that round the stair door is open
    And the GM sees "path_flag docks_shop.stairs open" and "actor_place mira" change
    And a change rule watching open has run in that round
    When Mira drops brass_key
    Then Mira can still take the stairs
    And the stair door is open from Mira's rooms too

  Scenario: Another actor walks through an opened door
    Given Mira opened the stair door
    When Aldric, who has no key, takes the stairs
    Then Aldric reaches Mira's rooms

  Scenario: A pack effect locks it again
    Given Mira opened the stair door
    When Aldric bolts the stairs, whose flag effect on its target sets open on path:docks_shop.stairs to false
    Then the GM's travel for Aldric up the stairs is refused NO_EXIT at once
    And it opens again only when someone holding brass_key is in the shop
