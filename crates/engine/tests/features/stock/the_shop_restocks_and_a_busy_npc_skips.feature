Feature: Habits keep the shop's items and leave the door note

  Scenario: The evening round leaves the door note and the oil
    When the evening round runs with Rowan away

  Scenario: Morning restock runs from Mira's rooms
    When the morning round runs with the note already left

  Scenario: Outside shop hours the oil stays
    When an off-hours round runs with the note already left

  Scenario: A player in the shop keeps Mira still
    When Rowan stands with Mira during shop hours

  Scenario: The GM restocks while Rowan is in the shop
    When the GM queues Mira's restock in front of Rowan

  Scenario: A queued door note sorts ahead of an unclaimed wait
    When the GM queues the door note ahead of a wait

  Scenario: A busy Mira does not restock
    When Mira is busy and a round runs

  Scenario: Fifty NPCs stay inside the round budget
    When fifty NPCs share one round

  Scenario: Mira's restock adds lamp_oil to docks_shop's items, capped by max
    When the morning round runs and the shelf already holds 6 lamp oil
    Then the shop holds 8 lamp oil
