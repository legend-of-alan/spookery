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
      | fault                         | file                         | code                | phrase        |
      | no audience                   | rules/moves.yaml             | SCHEMA              | no audience   |
      | unknown field                 | rules/moves.yaml             | SCHEMA              | unknown field |
      | unknown check                 | rules/moves.yaml             | SCHEMA              | not a check   |
      | duplicate move                | rules/moves.yaml             | SCHEMA              | listed twice  |
      | old format tag                | rules/moves.yaml             | SCHEMA              | no version    |
      | missing meters                | rules/meters.yaml            | NOT_IN_SPEC         | missing       |
      | pack_rev mismatch             | state/now.yaml               | PACK_REV_MISMATCH   | pack_rev      |
      | hidden fact in a place body   | world/places/docks_shop.md   | HIDDEN_FACT_IN_BODY | hidden fact   |
      | unknown place in a range      | cast/mira.md                 | SCHEMA              | not a place   |

  Scenario: Two problems in two files are both reported
    Given a docks table
    When the copy is broken by "no audience"
    And the copy is broken by "unknown place in a range"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/moves.yaml"
    And the report names "cast/mira.md"
    And stdout is empty
    And the pack bytes are unchanged

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
    When the copy is broken by "no audience"
    And the copy is broken by "unknown place in a range"
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And the JSON diagnostics are ordered by file, line, and path
    When the operator runs spook check --json again
    Then stdout matches the earlier JSON
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: The authoring example move passes check
    Given a docks table
    When the worked example row is appended to the moves file
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And the JSON array is one diagnostic code "PACK_REV_MISMATCH" file "state/now.yaml" path "pack_rev"
    When the operator runs migrate
    Then the command exits 0
    And stderr is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty

  Scenario: No file under .grok uses an absolute path
    Then no file under .grok contains an absolute path
