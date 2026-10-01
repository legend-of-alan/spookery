@v0
Feature: Tool scope
  Proof: v0_t09_tool_scope
  Tick-profile perceive with no as is UNSCOPED_PERCEIVE.

  Scenario: Tick-profile perceive with no as is UNSCOPED_PERCEIVE
    Given a temp copy of pack packs/oneshot-docks
    And the pack files are recorded
    Then tick-profile perceive with no as is UNSCOPED_PERCEIVE
    And tick-profile perceive as "" is UNSCOPED_PERCEIVE
    And tick-profile perceive as "gm" is UNSCOPED_PERCEIVE
    And tick-profile perceive as "system" is UNSCOPED_PERCEIVE
    And tick-profile perceive as "system:clock" is UNSCOPED_PERCEIVE
    And tick-profile perceive as "npc:" is UNSCOPED_PERCEIVE
    And tick-profile perceive as "player:pc_rowan" is UNSCOPED_PERCEIVE
    And tick-profile perceive as "npc:mira" with focus "stock" is SCHEMA
    And tick-profile perceive as "npc:mira" matches kernel perceive
    And tick-profile perceive as "npc:mira" with focus "place" matches kernel perceive
    And tick-profile perceive as "npc:mira" with focus "actor" matches kernel perceive
    And tick-profile perceive as "npc:mira" with budget 800 matches kernel perceive
    And kernel perceive for "npc:mira" still has a fact body
    And tick-profile perceive as "npc:mira" with budget 0 has the kernel fact count and empty fact bodies
    And tick-profile perceive as "npc:mira" with budget 0 keeps fact "fact_ledger_short" text "The lamp-oil ledger is short."
    And tick-profile perceive as "npc:mira" with budget 0 does not list unknown "fact_ledger_short"
    And tick-profile perceive as "npc:mira" with budget 0 lists unknown "fact_room_lamp"
    And the pack files are unchanged
