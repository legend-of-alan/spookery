Feature: A rule's effects are all or nothing

  Scenario: A rule whose second effect is refused leaves the first effect unapplied
    Given a docks row overreach that adds 1 lamp oil and then removes 8
    When Mira plays overreach
    Then lamp oil stays 4 and the round names OUT_OF_RANGE

  Scenario: A rule over the effect cap is refused, not truncated
    Given max_effects_per_intent is 1 and a row with 2 effects
    Then no effect from that row applies and the round log names the refusal

  Scenario: A refused roll is not reported as a success
    Given a docks row spilled that rolls grit and then removes 8 lamp oil
    When Mira plays spilled
    Then lamp oil stays 4, the round names OUT_OF_RANGE, and no roll is reported

  Scenario: A later kernel intent sees what the earlier one already did
    Given two kernel waits queued for Mira
    When those waits are played
    Then only the first wait lands and the second is INELIGIBLE
