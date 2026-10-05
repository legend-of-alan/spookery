Feature: A queued wait survives the seat

  Scenario: A wake does not drain a queued wait when the seat drops
    When a queued wait meets a wake and the seat drops

  Scenario: Sitting again keeps the same queued wait
    When the player drops and sits again with a queued wait

  Scenario: An early wake after a commit does not tick
    When a committed wait wakes early

  Scenario: A due wake applies the queued wait once
    When a queued wait is due
