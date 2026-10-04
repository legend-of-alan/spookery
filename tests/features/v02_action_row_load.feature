@v02
Feature: Action row load
  Proof: v02_p06_pack_load_rejects_a_row_that_is_not_one_flag
  Pack load accepts token none with on speaker, and token actor with on named, as one flag.
  Any other pair is SCHEMA at pack load.
  A flag template is class, key, and a boolean value, and key is not busy.
  A missing key or an extra key is SCHEMA.
  An empty effects list with no success list and no failure list is a yield.
  An empty success list or an empty failure list is SCHEMA.
  attack is not a reserved name.
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
    When moves.yaml is rewritten as "p06-busy"
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
    And move "work" audience is agenda
    And move "attack" is absent
    And move "attempt" is absent

  Scenario: An empty effects list with no outcome lists loads as a yield
    When moves.yaml is rewritten as "p06-empty"
    Then move "mark" effects are empty
