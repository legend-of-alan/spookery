@v0
Feature: Budget
  Proof: v0_t08_budget
  50 eligible NPCs, one tick, at most 8 act, at most 12 intents applied.

  Scenario: Fifty eligible NPCs on one tick act at most 8 and apply at most 12 intents
    Given a crowd pack of 50 eligible NPCs using packs/oneshot-docks rules
    When the crowd pack is ticked once
    Then tick_index is 1
    And the shard has 50 NPCs
    And between 1 and 8 NPCs acted on tick 1
    And state/log/tick-0001.jsonl records between 1 and 12 applied intents
