@v02
Feature: Tool keys
  Proof: v02_p01_gm_tools_list_names_distinct_keys_and_keeps_the_refusals
  GM tools/list names the six sitting handlers. A missing required key stays SCHEMA or UNSCOPED_PERCEIVE. npc_act stays LLM_DISABLED. session_open, patch_entity, resolve_check, apply_effect, recall, and session_save stay NOT_IN_SPEC. A name off the list stays NOT_IN_SPEC.

  Scenario: Tool keys list names distinct keys and keeps the refusals
    Given a temp copy of pack packs/oneshot-docks
    When the GM lists tools on the tool-keys shard
    Then the tool-keys names are session_status, perceive, intent, say, npc_act, commit_turn
    And the tool-keys list does not include patch_entity
    And each tool-keys description contains its recorded phrase and the descriptions are distinct
    And the tool-keys say schema requires address and text and branches table, place, and actor
    And the tool-keys perceive schema requires as and accepts focus and budget_tokens
    And the tool-keys commit_turn schema requires include_tick and accepts session_id
    And the tool-keys session_status schema accepts since and requires nothing
    And the tool-keys intent schema requires actor_id, actor_kind, source, and move and source stays player
    And the tool-keys live schemas for say, perceive, commit_turn, session_status, and intent differ
    And the tool-keys schema for npc_act is type object
    And tool-keys say returns SCHEMA when a required key is missing or from is supplied
    And tool-keys perceive returns UNSCOPED_PERCEIVE when as is missing
    And tool-keys commit_turn returns SCHEMA when include_tick is missing
    And tool-keys intent returns SCHEMA when actor_id, actor_kind, source, or move is missing
    And tool-keys session_status with an empty object has queued and last_seq and no code and no events
    And tool-keys session_status with an extra key returns SCHEMA
    And tool-keys npc_act returns LLM_DISABLED
    And tool-keys session_open, patch_entity, resolve_check, apply_effect, recall, session_save, and not_a_tool return NOT_IN_SPEC
    And the tool-keys now.yaml is unchanged and session.jsonl is absent
    And the tool-keys shard stops
