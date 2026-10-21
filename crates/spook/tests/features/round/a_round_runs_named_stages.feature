Feature: A round runs named stages

  Scenario: Stages run in order
    When a docks round is played
    Then the round followed the standard step order
