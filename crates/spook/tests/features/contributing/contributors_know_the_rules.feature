Feature: Contributors know the rules

  Scenario: Files exist
    Then CONTRIBUTING.md states issues only
    And .github/ISSUE_TEMPLATE holds bug, pack and feature forms
