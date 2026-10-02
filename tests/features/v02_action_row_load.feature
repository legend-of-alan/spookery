@v02
Feature: Action row load
  Proof: v02_p06_pack_load_rejects_a_row_that_is_not_one_flag
  Pack load accepts token none with on speaker, and token actor with on named, as one flag.
  Any other pair is SCHEMA at pack load.
  effects is a non-empty list of class, key, and a boolean value, and key is not busy.
  An empty list, a missing key, or an extra key is SCHEMA.
  attack with an effects list is SCHEMA.
  A check key loads on the row.
  Travel keeps token place, on speaker, and a location effect.

  Scenario: Action row load rejects a row that is not one flag
    When moves.yaml is rewritten as "p06-location-key"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-location-value"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-location-other"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-place-flag"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-place-named"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-actor-location"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-none-location"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-actor-speaker"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-none-named"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-travel-flag"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-attack"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-busy"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-empty"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-missing"
    Then pack load fails with SCHEMA
    When moves.yaml is rewritten as "p06-extra"
    Then pack load fails with SCHEMA

  Scenario: Action row load keeps check grit on the mark flag row
    When moves.yaml is rewritten as "p06-check"
    Then move "mark" check is "grit"
    And move "mark" token is actor
    And move "mark" on is named
    And move "mark" effects are one flag "marked" true

  Scenario: Action row load accepts mark and wave and keeps the travel place row
    When moves.yaml is rewritten as "p06-ok"
    Then move "mark" check is absent
    And move "mark" token is actor
    And move "mark" on is named
    And move "mark" effects are one flag "marked" true
    And move "wave" token is none
    And move "wave" on is speaker
    And move "wave" effects are one flag "marked" false
    And move "travel" token is place
    And move "travel" on is speaker
    And move "travel" effects are location
    And move "work" has no action row
    And move "attack" has no action row
    And move "attempt" check is absent
