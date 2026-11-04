Feature: A pack can turn idle ticking off

  Scenario: An empty table past its interval writes no round while idle is off
    When idle is off and an empty table wakes after 360 seconds

  Scenario: A long absence writes no catch-up while idle is off
    When idle is off and an empty table wakes after 3700 seconds

  Scenario: A seated line and spook round still advance while idle is off
    When idle is off and a player waits and spook round runs

  Scenario: The GM commit still advances while idle is off
    When idle is off and the GM commits a queued wait
