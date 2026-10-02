@v0
Feature: MCP profiles
  Proof: v0_pack_tick_and_studio_tools_follow_section_8
  pack-tick and pack-studio tools/list match the recorded names. spook run --mcp pack-gm fails and prints NOT_IN_SPEC.

  Scenario: pack-tick and pack-studio list their tools and pack-gm is NOT_IN_SPEC
    Given a temp copy of pack packs/oneshot-docks
    When spook run lists tools for --mcp pack-tick
    Then tools/list order is session_status, perceive, recall, npc_act, resolve_check, apply_effect, tick_commit
    When spook run lists tools for --mcp pack-studio
    Then tools/list order is pack_info, read_entity, patch_entity, lint_pack, simulate_knowledge, run_conformance, compile_preview
    When spook run starts with --mcp pack-gm
    Then the pack-gm command fails
    And the pack-gm stderr is NOT_IN_SPEC
