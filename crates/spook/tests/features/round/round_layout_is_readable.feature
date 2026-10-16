Feature: Round layout is readable

  Scenario: Public API unchanged
    Given the round crate after the move
    Then spook builds without import changes

  Scenario: Replay still matches
    Then spook verify passes on the existing feature packs
