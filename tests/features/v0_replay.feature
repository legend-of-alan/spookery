@v0
Feature: Replay
  Proof: v0_t07_replay
  Twenty kernel-only ticks replay onto the start snapshot as a byte-equal now.yaml, and the source pack and tape stay unchanged.

  Scenario: Twenty kernel-only ticks replay to a byte-equal now.yaml
    Given a temp start copy and a played copy of pack packs/oneshot-docks
    When the played pack is kernel-ticked 20 times and replayed onto the start snapshot
    Then the replay now.yaml bytes equal the golden now.yaml
    And the replay directory is not the played pack
    And the played now.yaml bytes still equal the golden now.yaml
    And the played tick log tick-0020.jsonl bytes are unchanged
