Feature: The shop restocks and a busy NPC skips

  Scenario: Mira restocks oil while Rowan is away
    When Mira is in the shop and Rowan is on the street

  Scenario: A busy Mira does not act and Bram does
    When Mira is busy and a tick runs

  Scenario: Fifty minor NPCs stay inside the tick budget
    When fifty minor NPCs share one tick
