Feature: Talk at an empty table appends nothing

  Scenario: Talk while nobody is seated
    Given a docks table
    When talk to mira is submitted on an empty table saying "good evening"
    Then the tape file is absent
