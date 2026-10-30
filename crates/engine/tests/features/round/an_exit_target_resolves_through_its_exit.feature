Feature: An exit target resolves through its exit

  Scenario: Walking through an exit toward a seated player counts as touching them
    Given the docks pack with Rowan seated in the shop
    When Bram and Aldric wait and then Mira travels through the "stairs" exit
    Then Mira's travel is picked ahead of both waits
    And spook verify passes on the docks pack

  Scenario: A teleport cannot be aimed at an exit name
    Given a shop rule that teleports to its exit target
    When Rowan aims it at the "docks_street" exit
    Then it is refused with SCHEMA and Rowan stays in the shop
    And spook verify passes on the docks pack
