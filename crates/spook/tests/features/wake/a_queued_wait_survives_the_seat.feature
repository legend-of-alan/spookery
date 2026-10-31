Feature: A queued wait survives the seat

  Scenario: A queued wait outlasts wakes while the player drops and sits again
    When a queued wait meets wakes while the player drops and sits again

  Scenario: An early wake after a commit does not play a round
    When a committed wait wakes early

  Scenario: A due wake applies the queued wait once
    When a queued wait is due
