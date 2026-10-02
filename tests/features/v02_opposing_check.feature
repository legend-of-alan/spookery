@v02
Feature: Opposing check
  Proof: v02_p12_a_queued_action_supplies_the_opposing_check
  On the tick that applies a row that names a check, an action the named actor queued before commit_turn supplies the opposing check. That check is resolved for the named actor at target 0, and its total is the target of the row being applied. A row with no such queued action is still resolved at target 0. The acting player sees a check success of true or false.

  Scenario: Opposing check with no queued defend resolves the mark at target 0
    Given the oppose port is locked
    And an oppose pack labeled "oppose-open"
    Then the oppose pack opens at state_rev 0 world_time 1080 tick_index 0 with Rowan and Nell in docks_shop and Nell has no flags
    When the oppose pack is running with a GM and a player
    Then the oppose shard is still at world_time 1080 with no tick log
    When the player marks pc_nell on the oppose shard
    Then the oppose mark line is queued and the roll has not happened
    And oppose session status lists Rowan mark and no defend at world_time 1080
    When the GM commits the oppose turn
    Then the player sees oppose check success true and no faces
    And the open oppose tick resolved one check and marked Nell

  Scenario: Opposing check from a queued defend sets the mark target
    Given the oppose port is locked
    And an oppose pack labeled "oppose-held"
    Then the oppose pack opens at state_rev 0 world_time 1080 tick_index 0 with Rowan and Nell in docks_shop and Nell has no flags
    When the oppose pack is running with a GM and a player
    Then the oppose shard is still at world_time 1080 with no tick log
    When the GM queues oppose defend for pc_nell
    Then oppose defend is queued and the shard has not ticked
    When the player marks pc_nell on the oppose shard
    Then the oppose mark line is queued and the roll has not happened
    And oppose session status lists Nell defend and Rowan mark at world_time 1080
    When the GM commits the oppose turn
    Then the player sees oppose check success false and no faces
    And the held oppose tick resolved both checks and missed Nell
