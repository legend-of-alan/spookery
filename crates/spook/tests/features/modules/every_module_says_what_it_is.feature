Feature: Every module says what it is

  Scenario: Missing module doc fails
    Given a source file with no module doc
    Then the module-doc check fails naming that file

  Scenario: Clean tree passes
    Then the module-doc check passes on main
