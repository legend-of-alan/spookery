Feature: One action lifecycle

  Every move but say starts and finishes through one path. One that takes no ticks finishes
  in the round it starts. A walk that takes ticks puts the walker on the exit until they arrive.

  Background:
    Given a docks table with Rowan and Wren seated in the shop

  Scenario: An instant action lands this round and uses the act
    When Rowan plays "take lamp_oil"
    Then Rowan holds lamp_oil
    And Rowan has used the act of round 1

  Scenario: A walker is on the stairs and in neither room
    Given the stairs take 10 ticks and Rowan holds the brass key
    When Rowan plays "travel stairs"
    Then Rowan is at exit:docks_shop.stairs
    And Wren's look does not show Rowan
    And Rowan's look shows "on the stairs until tick 1090"
    And spook verify passes on this pack
    When rounds pass until tick 1090
    Then Rowan is at mira_rooms

  Scenario: A save with a walker on an exit the pack lacks is refused
    Given Rowan stands on exit:docks_shop.ladder in the save
    Then spook check refuses Rowan's place

  Scenario: A timed teleport lands like travel
    Given the rule "blink" teleports Rowan to mira_rooms and takes 10 ticks
    When Rowan plays "blink"
    Then Rowan is at docks_shop
    And Rowan's look shows "blinking until tick 1090"
    When rounds pass until tick 1090
    Then Rowan is at mira_rooms

  Scenario: A cooldown keeps the actor busy after the finish
    Given the rule "chant" takes 5 ticks and has a cooldown of 3 ticks
    When Rowan plays "chant"
    Then Rowan's look shows "recovering until tick 1088"
    And Rowan's "wait" is refused BUSY at once
    When rounds pass until tick 1090
    Then Rowan's "wait" lands

  Scenario: A free instant drink does not use the act
    Given the rule "drink" does not count its start as an act
    When Rowan plays "drink"
    Then that round refused nothing
    And Rowan has used no act

  Scenario: A free instant drink and a move both land in one round
    Given the rule "drink" does not count its start as an act
    When Rowan plays "drink" and "take lamp_oil" in the same round
    Then that round refused nothing
    And Rowan holds lamp_oil
    And Rowan has used the act of round 1

  Scenario: A habit the GM queues is checked like one the engine picks
    When the GM queues Mira's "restock_lamp_oil" outside the hours its if allows
    Then the GM is refused at once
    And a round later the shop still holds 4 lamp_oil
