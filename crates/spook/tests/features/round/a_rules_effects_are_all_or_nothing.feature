Feature: A rule's effects are all or nothing

  Scenario: A rule whose second effect is refused leaves the first effect unapplied
    Then the first effect is absent from the save and the tick log names the refusal

  Scenario: A rule over the effect cap is refused, not truncated
    Given max_effects_per_intent is 1 and a row with 2 effects
    Then no effect from that row applies and the tick log names the refusal
