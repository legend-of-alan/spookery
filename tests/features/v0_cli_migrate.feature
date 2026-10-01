@v0
Feature: CLI migrate
  Proof: v0_cli_migrate_bumps_state_rev_and_appends_evidence, v0_cli_migrate_keeps_a_hand_edit_and_rewrites_pack_rev, v0_cli_migrate_schema_failure_writes_nothing
  Two migrates bump state_rev and append evidence. A hand edit is kept and pack_rev is rewritten. Schema failure writes nothing.

  Scenario: Two migrates bump state_rev and append evidence
    Given a temp copy of pack packs/oneshot-docks
    When spook migrate runs
    And spook migrate runs
    Then both migrate commands exit 0
    And the first migrate stdout and stderr are empty
    And the second migrate stdout is empty
    And state_rev is 2
    And tick_index is 0
    And world_time is 1080
    And last_tick_at is 2026-01-01T00:00:00Z
    And now pack_rev is the docks pack rev
    And actor pc_rowan place is docks_shop
    And docks_shop lamp_oil is 4
    And state/log/tick-0001.jsonl does not exist
    And state/.txn does not exist
    And state/log/studio.jsonl is the two migrate evidence lines

  Scenario: Migrate keeps a hand edit and rewrites pack_rev
    Given a temp copy of pack packs/oneshot-docks
    When now.yaml has the migrate hand edit
    And a newline is appended to cast/mira.md
    And spook migrate runs
    Then the migrate command exits 0
    And the canon pack_rev differs from the docks pack rev
    And now pack_rev equals the canon pack_rev
    And state_rev is 1
    And world_time is 7
    And tick_index is 0
    And last_tick_at is 2026-01-01T00:00:00Z
    And actor pc_rowan place is mira_rooms
    And state/log/tick-0001.jsonl does not exist
    And state/log/studio.jsonl is the hand-edit migrate evidence

  Scenario: Migrate schema failure writes nothing
    Given a temp copy of pack packs/oneshot-docks
    And state/now.yaml is replaced with the schema failure document
    And the cli pack files are recorded
    When spook migrate runs
    Then migrate exits 1 with stderr SCHEMA
    And schema failure leaves the cli pack files unchanged
    And state/log/studio.jsonl does not exist
