Feature: Status writes nothing

  Scenario: Status on a docks table leaves the pack unchanged
    Given a docks table
    When the operator runs status
    Then status exits 0
    And the pack bytes are unchanged
