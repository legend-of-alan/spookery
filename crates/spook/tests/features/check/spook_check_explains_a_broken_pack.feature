Feature: spook check explains a broken pack

  Scenario: A copy of the docks pack passes
    Given a docks table
    When the operator runs spook check
    Then the check exits 0
    And stdout says "no problems"
    And stderr is empty
    And the pack bytes are unchanged

  Scenario Outline: A broken copy names the fault
    Given a docks table
    When the copy is broken by "<fault>"
    And the operator runs spook check
    Then the check exits 1
    And the report names "<file>"
    And the report names "<code>"
    And the report says "<phrase>"
    And the report has a location for "<file>"
    And stdout is empty
    And the pack bytes are unchanged

    Examples:
      | fault                         | file                         | code                | phrase              |
      | missing when                  | rules/verbs.yaml             | SCHEMA              | missing field 'when' |
      | unknown field                 | rules/verbs.yaml             | SCHEMA              | unknown field       |
      | unknown check                 | rules/verbs.yaml             | SCHEMA              | not a check         |
      | duplicate move                | rules/verbs.yaml             | SCHEMA              | listed twice        |
      | old format tag                | rules/verbs.yaml             | SCHEMA              | no version          |
      | missing meters                | rules/meters.yaml            | NOT_IN_SPEC         | missing             |
      | pack_rev mismatch             | state/save.yaml              | PACK_REV_MISMATCH   | pack_rev            |
      | exit to an unknown place      | world/places/docks_shop.md   | SCHEMA              | not a place         |
      | duplicate exit                | world/places/docks_shop.md   | SCHEMA              | listed twice        |

  Scenario: Two problems in two files are both reported
    Given a docks table
    When the copy is broken by "missing when"
    And the copy is broken by "exit to an unknown place"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/verbs.yaml"
    And the report names "world/places/docks_shop.md"
    And stdout is empty
    And the pack bytes are unchanged

  Scenario Outline: A repeated fact points at its own row
    Given a docks table
    When the copy is broken by "<fault>"
    And the operator runs spook check --json
    Then the check exits 1
    And a JSON diagnostic has code "SCHEMA" file "<file>" path "facts[1].id" at line <line> column 5
    And stderr is empty
    And the pack bytes are unchanged

    Examples:
      | fault                  | file                       | line |
      | duplicate place fact   | world/places/docks_shop.md | 9    |
      | duplicate cast fact    | cast/pc-rowan.md           | 10   |

  Scenario: A clean pack prints an empty JSON array
    Given a docks table
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: A broken pack prints JSON diagnostics
    Given a docks table
    When the copy is broken by "missing meters"
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And a JSON diagnostic has code "NOT_IN_SPEC" file "rules/meters.yaml" path "schema" and null line and column
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: JSON diagnostics stay in the same order
    Given a docks table
    When the copy is broken by "missing when"
    And the copy is broken by "exit to an unknown place"
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And the JSON diagnostics are ordered by file, line, and path
    When the operator runs spook check --json again
    Then stdout matches the earlier JSON
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: An unreachable place is a warning and the check still exits 0
    Given a docks table
    When the street exit is removed
    And the save hash is restamped
    And the operator runs spook check
    Then the check exits 0
    And the report names "WARNING"
    And the report says "no path"
    And stdout is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout is a JSON array of diagnostics
    And a JSON diagnostic has code "WARNING" file "world/places" path "docks_street" and null line and column
    And stderr is empty

  Scenario: Teleport in verbs.yaml is a warning and the check still exits 0
    Given a docks table
    When travel in verbs.yaml teleports
    And the save hash is restamped
    And the operator runs spook check
    Then the check exits 0
    And the report names "WARNING"
    And the report says "teleports"
    And stdout is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout is a JSON array of diagnostics
    And a JSON diagnostic has code "WARNING" file "rules/verbs.yaml" path "rules[1]" and null line and column
    And stderr is empty

  Scenario: The authoring example move passes check
    Given a docks table
    When the worked example row is appended to the moves file
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And the JSON array is one diagnostic code "PACK_REV_MISMATCH" file "state/save.yaml" path "pack_rev"
    When the operator runs migrate
    Then the command exits 0
    And stderr is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty
