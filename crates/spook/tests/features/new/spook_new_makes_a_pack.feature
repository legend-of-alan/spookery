Feature: spook new makes a pack

  Scenario: A new pack passes check
    When the operator runs spook new in a fresh directory
    Then the command exits 0
    And stdout names the next commands
    And stderr is empty
    And the written files match the starter template
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty

  Scenario: spook new on a non-empty directory changes nothing
    Given a non-empty directory
    When the operator runs spook new on that directory
    Then the command exits 1
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: spook new refuses a missing parent
    When the operator runs spook new under a missing parent
    Then the command exits 1
    And stdout is empty
    And stderr names the missing parent
    And the missing parent was not created

  Scenario: The generated pack runs
    When the operator runs spook new in a fresh directory
    And play joins that pack on a free loopback address
    Then play shows "You are in Yard."
    When play types "travel lane"
    Then play shows "You are in Lane."
    And the travel committed a round
    When the operator runs spook new beside that pack
    And the operator runs tick on that untouched pack
    Then the keeper habit has set tended

  Scenario: The first-pack guide passes check
    When the operator runs spook new in a fresh directory
    And the first-pack guide steps are applied in order
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty

  Scenario: spook new writes nothing outside the directory
    When the operator runs spook new beside a marker file
    Then the command exits 0
    And the marker file is unchanged
    And the parent gained only the new pack
    And the starter template is unchanged

  Scenario: The starter template passes check
    When the operator runs spook check --json on the starter template
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty
