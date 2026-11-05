Feature: A timed action finishes against its target

  Scenario: A ten-tick verb owned by the person it targets still finds its rule when it finishes
    Given a docks pack where Aldric's cast file holds a ten-tick verb "ask" that lowers the target's rest
    When Rowan asks Aldric and the table plays until the action finishes
    Then Aldric's rest is 2

  Scenario: A finish that is refused is journaled and the round still commits
    Given a docks pack whose verbs hold a ten-tick verb "haul" that takes 8 lamp oil from the actor
    When Rowan hauls with no lamp oil and the table plays until the action finishes
    Then the finishing round commits with arrival:pc_rowan refused MISSING_ITEM, Rowan is no longer busy, and spook verify passes
