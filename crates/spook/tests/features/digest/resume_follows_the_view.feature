Feature: Resume follows the view

  Scenario: A note the view hides is not in the player's resume
    Given the docks pack where Mira's door note carries the hidden fact "fact_ledger_short"
    When thirteen ticks pass
    Then the player resume for "pc_rowan" does not list "note_mira_door"
    And the perceive view for "pc_rowan" does not list "note_mira_door"
