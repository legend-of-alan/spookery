Feature: Cancelling your own action

  A player may cancel their own timed action when its rule is cancellable. The action is
  dropped with no refund and its finish never runs. Any other cancel is refused, and the
  action still lands.

  Background:
    Given a docks table with Rowan and Wren seated in the shop

  Scenario: A player cancels a cancellable chant and nothing else
    Given the rule "chant" takes 20 ticks, is cancellable and costs 1 rest
    And the rule "brew" takes 20 ticks
    When Rowan plays "chant"
    And Rowan plays "cancel"
    Then "chant" never finished and Rowan paid 1 rest
    And Rowan's "wait" lands
    When Rowan plays "brew"
    Then Rowan's "cancel" is refused INELIGIBLE at once
    When rounds pass until tick 1115
    Then "brew" finished
    And "chant" never finished and Rowan paid 1 rest
    And spook verify passes on this pack

  Scenario: A walk cannot be cancelled
    Given the rule "travel" is cancellable
    And the stairs take 10 ticks and Rowan holds the brass key
    When Rowan plays "travel stairs"
    Then Rowan's "cancel" is refused INELIGIBLE at once
    When rounds pass until tick 1090
    Then Rowan is at mira_rooms

  Scenario: A free player's cancel is refused
    Then Rowan's "cancel" is refused INELIGIBLE at once

  Scenario: A pack verb named cancel is refused
    Given the rule "cancel" takes 20 ticks
    Then spook check refuses the verb "cancel"
