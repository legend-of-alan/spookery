Feature: One writer per pack

  Scenario: spook tick is refused while spook run holds the pack
    Given spook run is serving a docks table
    When the host runs spook tick on that pack
    Then spook tick exits non-zero with COMMIT_CONFLICT
    And the journal is unchanged

  Scenario: spook migrate is refused while spook run holds the pack
    Given spook run is serving a docks table
    When the host runs spook migrate on that pack
    Then spook migrate exits non-zero with COMMIT_CONFLICT
    And the journal is unchanged

  Scenario: the table keeps a verifiable journal after a refused tick
    Given spook run is serving a docks table
    And spook tick was refused
    When a seated player commits a move
    And spook run stops
    Then spook verify passes

  Scenario Outline: spook <command> on a path that is not a pack says NOT_IN_SPEC
    When the host runs spook <command> on a missing pack
    Then spook <command> exits non-zero with NOT_IN_SPEC

    Examples:
      | command |
      | tick    |
      | migrate |
