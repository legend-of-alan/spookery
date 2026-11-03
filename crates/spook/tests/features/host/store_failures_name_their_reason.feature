Feature: Store failures name their reason

  Scenario: A tape the disk will not write is STORE_IO
    Given a docks table with a seated player
    When the tape cannot be written
    And the player tries to say "hello"
    Then the player is refused with "STORE_IO"
    And no round record was written

  Scenario: A journal bundle the disk will not read is STORE_IO
    Given a docks table with a seated player
    When the player waits
    And the live journal bundle cannot be read
    And the player waits
    Then the player is refused with "STORE_IO"

  Scenario: A journal record changed under the table is JOURNAL_DIVERGED
    Given a docks table with a seated player
    When the player waits
    And the last journal record is changed under the table
    And the player waits
    Then the player is refused with "JOURNAL_DIVERGED"
