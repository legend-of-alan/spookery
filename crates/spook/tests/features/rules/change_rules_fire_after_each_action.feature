Feature: Change rules fire after each action

  A trigger pass runs right after each start and each finish lands, so every action's change
  fires its watchers and a rule can react before the next action in the round.
  `max_trigger_depth` limits each pass, not the round.

  Background:
    Given a docks table with Rowan, Wren and Harbormouse seated in the shop

  Scenario: A door opened twice in one round rings the bell twice
    Given a rule rings the bell whenever the cellar door opens
    When Rowan opens the door, Wren closes it and Harbormouse opens it in the same round
    Then the bell rings twice this round

  Scenario: A rule reacts before the next action in the round
    Given a rule locks the gate when Rowan arrives in the street
    When Rowan plays "go_street" and Wren plays "go_gate" in the same round
    Then Wren is refused because the gate is locked

  Scenario: Each action's pass gets the whole trigger depth
    Given the trigger depth is 1 and two change rules, the second's if reading the first's flag
    When Rowan plays "pull" and Wren plays "push" in the same round
    Then both rules ran and the round logged no trigger_depth
