@v01
Feature: Typed GM client
  Proof: v01_typed_gm_line_does_not_call_say
  Human spook gm without --stdio exits NOT_IN_SPEC. It writes no session.jsonl and leaves now.yaml unchanged.

  Scenario: Typed GM without --stdio exits NOT_IN_SPEC
    Given a listening typed GM shard
    When spook gm runs without --stdio
    Then the typed GM stderr is NOT_IN_SPEC
    And the typed GM exits 1
    And the typed GM pack has no session.jsonl
    And the typed GM now.yaml is unchanged
    And the typed GM shard stops
