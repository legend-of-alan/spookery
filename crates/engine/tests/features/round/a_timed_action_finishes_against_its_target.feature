Feature: A timed action finishes against its target

  Scenario: A ten-minute verb owned by the person it targets still finds its rule when it finishes
    Given a docks pack where Aldric's cast file holds a ten-minute verb "ask" that lowers the target's rest
    When Rowan asks Aldric and the table plays until the action finishes
    Then Aldric's rest is 2
