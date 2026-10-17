Feature: A round runs named stages

  Scenario: Behaviour unchanged
    Given every existing cucumber feature
    Then all pass and spook verify passes on both sample packs

  Scenario: One dice path
    Then crates/round has exactly one call site of roll

  Scenario: Stages run in order
    When a docks round is played
    Then the round followed the standard step order
