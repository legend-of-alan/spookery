@v01
Feature: GM allowlist
  Proof: v01_gm_tools_call_is_not_in_spec, v01_no_cargo_feature_named_llm, v01_p12_gm_tools_reach_perceive_and_the_refusals_write_nothing
  GM tools/call for perceive reaches the handler. npc_act returns LLM_DISABLED. patch_entity from the GM seat returns NOT_IN_SPEC. GM session_open returns NOT_IN_SPEC and writes nothing.
  Typed stdin is outside this feature.

  Scenario: GM allowlist empty tools/call returns the recorded codes
    Given a listening GM allowlist shard
    When the GM lists tools and calls the empty-argument tools, commit_turn, and the pc_rowan wait intent
    Then the listed GM tools are session_open, session_status, perceive, intent, say, resolve_check, apply_effect, npc_act, commit_turn, recall, session_save
    And empty session_open returns NOT_IN_SPEC
    And empty say returns SCHEMA
    And empty perceive returns UNSCOPED_PERCEIVE
    And empty npc_act returns LLM_DISABLED
    And empty patch_entity returns NOT_IN_SPEC
    And empty commit_turn returns SCHEMA
    And the pc_rowan wait intent is queued and has no code
    And the GM allowlist now.yaml is unchanged
    And the GM allowlist pack has no tick file
    And the GM allowlist pack has no session.jsonl
    And the GM allowlist shard stops

  Scenario: GM allowlist cargo manifests do not contain llm
    Then Cargo.toml, src/kernel/Cargo.toml, src/mcp/Cargo.toml, and src/tick/Cargo.toml do not contain llm

  Scenario: GM allowlist perceive reaches the handler and refusals write nothing
    Given a listening GM allowlist shard
    When the GM raw session lists tools and calls perceive, npc_act, session_open, patch_entity, and the refusal names
    Then the raw listed GM tools are session_open, session_status, perceive, intent, say, resolve_check, apply_effect, npc_act, commit_turn, recall, session_save
    And every listed tool description contains "queued until commit_turn"
    And every listed tool description contains "does not tick"
    And every listed tool description contains "player:<id>"
    And every listed tool description contains "npc:<id>"
    And every listed tool description contains "session_open"
    And every listed tool description contains "patch_entity"
    And every listed tool description contains "since"
    And every listed tool description contains "player speech"
    And every listed tool description contains "the queue"
    And perceive as player:pc_rowan is kind perceive with a place, no code, and no spook/event
    And perceive as npc:mira is kind perceive for that viewer
    And perceive as gm returns UNSCOPED_PERCEIVE
    And perceive with an empty object returns UNSCOPED_PERCEIVE
    And npc_act for mira returns only LLM_DISABLED
    And session_open for s-gm as player:pc_rowan returns NOT_IN_SPEC
    And patch_entity of cast/mira.md returns NOT_IN_SPEC and not method-not-found
    And resolve_check, apply_effect, recall, session_save, and not_a_tool return NOT_IN_SPEC and not method-not-found
    And the GM allowlist now.yaml is unchanged
    And the GM allowlist pack has no session.jsonl
    And cast/mira.md does not contain rewritten
    And the GM allowlist shard stops
