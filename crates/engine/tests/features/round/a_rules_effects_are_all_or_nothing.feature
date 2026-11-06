Feature: A rule's effects are all or nothing

  Scenario: A rule whose second effect is refused leaves the first effect unapplied
    Given a docks row overreach that adds 1 lamp oil and then removes 8
    When Mira plays overreach
    Then lamp oil stays 4 and the round names OUT_OF_RANGE

  Scenario: A refused rule leaves the flags it cleared and set, the door it opened and the modifier it applied as they were
    Given a docks row unbar that clears Mira's calm, opens the stair door, sets calm again, braces her grit by 2 and then removes 8 lamp oil
    When Mira plays unbar
    Then lamp oil stays 4, the round names OUT_OF_RANGE, the stair door stays locked, and Mira keeps calm and wary, no brace and her grit

  Scenario: A rule over the effect cap is refused, not truncated
    Given max_effects_per_intent is 1 and a row with 2 effects
    Then no effect from that row applies and the round log names the refusal

  Scenario: A change rule over the effect cap is refused whole, and the log records it
    Given max_effects_per_intent is 1 and a street rule that sets 2 flags when Rowan arrives
    When Rowan walks to the street
    Then neither flag is set and the round log names the change rule's BUDGET refusal

  Scenario: A refused roll is not reported as a success
    Given a docks row spilled that rolls grit and then removes 8 lamp oil
    When Mira plays spilled
    Then lamp oil stays 4, the round names OUT_OF_RANGE, and no roll is reported

  Scenario: A later intent sees what the earlier one already did
    Given the docks pack, where Mira's door-note habit is due
    When the GM queues mira wait and an empty round plays
    Then the wait lands, and Mira's habit is refused INELIGIBLE because she already acted this round
