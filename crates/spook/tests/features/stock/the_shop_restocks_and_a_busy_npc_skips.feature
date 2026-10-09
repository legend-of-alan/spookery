Feature: Habits keep the shop and leave the door note

  Scenario: The evening tick leaves the door note and the oil
    When the evening tick runs with Rowan away

  Scenario: Morning restock runs from Mira's rooms
    When the morning tick runs with the note already left

  Scenario: Outside shop hours the oil stays
    When an off-hours tick runs with the note already left

  Scenario: A player in the shop keeps Mira still
    When Rowan stands with Mira during shop hours

  Scenario: The GM restocks while Rowan is in the shop
    When the GM queues Mira's restock in front of Rowan

  Scenario: A queued door note sorts ahead of an unclaimed wait
    When the GM queues the door note ahead of a wait

  Scenario: A busy Mira does not restock
    When Mira is busy and a tick runs

  Scenario: Fifty NPCs stay inside the tick budget
    When fifty NPCs share one tick
