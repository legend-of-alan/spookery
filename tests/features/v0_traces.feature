@v0
Feature: Tick jsonl traces
  Proof: v0_trace_a_empty_tick_jsonl, v0_trace_b_flush_jsonl, v0_trace_c_budget_jsonl
  An idle docks tick, a flushed player wait, and a five-NPC budget tick each write one jsonl line.

  Scenario: Empty docks tick jsonl matches trace A
    Given a temp copy of pack packs/oneshot-docks
    And mira and bram are ineligible and the pack rev is rewritten
    When the kernel ticks this pack once
    Then tick 1 jsonl equals the empty docks trace

  Scenario: Flushed player wait jsonl matches trace B
    Given a gate pack with Rowan present at world_time 0
    When the queued player wait is committed
    Then tick 1 jsonl equals the flushed wait trace

  Scenario: Five-NPC budget tick jsonl matches trace C
    Given a yard pack of five eligible NPCs with four work needs
    When the kernel ticks this pack once
    Then tick 1 jsonl equals the five-NPC budget trace
