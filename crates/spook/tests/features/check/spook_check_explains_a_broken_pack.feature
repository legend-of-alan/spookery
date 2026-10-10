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
