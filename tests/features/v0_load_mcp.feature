@v0
Feature: Load MCP
  Proof: v0_c01_load_mcp
  spook run pack --mcp pack-table checks tools/list order. List-only. No say.

  Scenario: pack-table tools/list returns the table order and does not include say
    Given a temp copy of pack packs/oneshot-docks
    When spook run starts with --mcp pack-table
    Then tools/list order is session_open, session_status, perceive, intent, resolve_check, apply_effect, npc_act, commit_turn, recall, session_save
    And tools/list does not include say
