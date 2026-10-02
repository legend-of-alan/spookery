@v01
Feature: Typed GM client
  Proof: v01_typed_gm_line_does_not_call_say
  Human spook gm without --stdio prints NOT_IN_SPEC for "say from the keyboard", writes no session.jsonl, and leaves now.yaml unchanged. The gm process stays running.

  Scenario: Typed GM say from the keyboard prints NOT_IN_SPEC
    Given a listening typed GM shard
    When the typed GM sends say from the keyboard
    Then the typed GM stderr is NOT_IN_SPEC
    And the typed GM pack has no session.jsonl
    And the typed GM now.yaml is unchanged
    And the typed GM process is still running
    And the typed GM shard stops
