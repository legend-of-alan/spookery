Feature: Travel follows the map

  Scenario: No exit, no travel
    Given docks_street has no exit to mira_rooms
    When rowan travels to mira_rooms from docks_street
    Then the intent is rejected with NO_EXIT and rowan stays put

  Scenario: Locked exit
    Given the stairs exit needs brass_key and rowan has none
    When rowan travels via stairs
    Then rowan sees "The stair door is locked." and no state changes

  Scenario: Exit cost
    When rowan travels via stairs holding brass_key
    Then rowan is in mira_rooms and the 2 minute cost is stamped on the arrival

  Scenario: Hidden exit
    Given the trapdoor is hidden and rowan does not know fact_trapdoor
    Then rowan's perceive and help do not list the trapdoor

  Scenario: Explicit teleport
    Given a place rule with location to cellar, teleport true
    Then the PC reaches cellar with no exit

  Scenario: With no GM, a lone traveller arrives on the travel line
    Given the stairs exit costs 10 minutes
    When rowan travels via stairs at 21:30
    Then the world is at 21:40 and rowan is in mira_rooms
    And rowan's line shows "mira_rooms"

  Scenario: Arrivals complete in time order
    Given rowan arrives at 21:40 and nell at 21:35
    Then nell's arrival is applied before rowan's, in separate ticks

  Scenario: Arrival is stamped at its exact minute and the actor acts next tick
    Given ticks are 5 minutes while seated and the stairs exit costs 2 minutes
    When rowan travels via stairs in the tick starting 21:30
    Then the tick still ends at 21:35
    And rowan's arrival is recorded with arrived_at 21:32 in that tick's line
    And a move from rowan is accepted from the tick starting 21:35

  Scenario: Ticks never shorten
    Then every tick line shows world_time_after minus world_time_before equal to 5 seated or 60 unseated

  Scenario: Fast-forward when all seated PCs are busy
    Given rowan and nell are seated and both busy, with no GM
    Then the shard ticks to 21:35 then 21:40 without waiting on the wall clock
    And an NPC habit due at 21:35 runs in that tick

  Scenario: GM mode waits for commit
    Given a GM is seated and both PCs are busy
    Then nothing advances until the GM commits

  Scenario: Fast-forward is capped
    Given busy_until is 200 ticks away and max_fastforward_ticks is 12
    Then 12 ticks run and the rest continues on the next action

  Scenario: Replay
    Then spook verify passes over a fast-forwarded journey

  Scenario: A player map lists visited places
    When rowan travels via stairs holding brass_key
    Then rowan's map lists mira_rooms and not the trapdoor
