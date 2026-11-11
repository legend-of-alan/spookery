Feature: Resume follows the view

  Scenario: A note the view hides is not in the player's resume
    Given the docks pack where Mira's door note carries the hidden fact "fact_ledger_short"
    When thirteen rounds pass
    Then the player resume for "rowan" does not list "note_mira_door"
    And the perceive view for "rowan" does not list "note_mira_door"
