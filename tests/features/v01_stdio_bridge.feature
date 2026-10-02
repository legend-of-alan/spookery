@v01
Feature: Stdio bridge
  Proof: v01_p21_stdio_bridge_copies_perceive_and_keeps_a_refused_hello_off_stdout
  spook gm --stdio copies a perceive tools/call to the shard and the handler runs. The bridge does not write now.yaml. After a player say, session_status with since 0 returns that tape object in events and last_seq at least 1. A refused hello is on the bridge stderr, and its stdout has no JSON-RPC.

  Scenario: Stdio bridge copies perceive and keeps a refused hello off stdout
    Given a listening shard with a player who said for the bridge
    When the stdio bridge calls perceive as player:pc_rowan
    Then the bridge perceive is kind perceive for viewer player:pc_rowan with a place
    And the bridge now.yaml matches the seated bytes
    When the stdio bridge calls session_status since 0
    Then session_status last_seq is at least 1 and events include say for the bridge
    And the bridge now.yaml matches the seated bytes
    And the stdio bridge is still running
    When a second stdio bridge hello is refused
    Then the refused stdio bridge exits 1 with SESSION_FULL on stderr and empty stdout
    And the bridge now.yaml matches the seated bytes
    And the stdio bridge shard stops
