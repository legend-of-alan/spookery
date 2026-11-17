Feature: Actions visibly matter

  Scenario: A player sees their own meters
    Given a docks table
    And harbormouse has a hidden meter
    When harbormouse looks at their view
    Then harbormouse sees their own hp and might
    And harbormouse does not see a hidden meter
