Feature: The debt clock does not run itself

  Scenario: Twelve empty ticks leave the debt paused
    When twelve empty ticks pass

  Scenario: A player cannot queue an agenda name
    When a player submits an agenda name
