Feature: Ctrl+C shuts the host down

  # The steps are unix only, so Windows skips this scenario.
  @allow.skipped
  Scenario: SIGINT stops the host cleanly, and closed stdin does not
    Given a docks table
    When spook run seats a player after its stdin closes
    And spook run receives SIGINT
    Then spook run exits 0 and spook verify passes
