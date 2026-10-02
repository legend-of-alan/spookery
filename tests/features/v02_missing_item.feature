@v02
Feature: Missing item
  Proof: v02_p13_a_sheet_that_lacks_the_named_item_queues_nothing
  A sheet that lacks the named item is MISSING_ITEM and queues nothing. A sheet that holds the item still runs the opposed row.

  Scenario: Missing item pack load rejects an empty item id and an unknown key
    Given an item moves rewrite labeled "p13-empty-item" with an empty item id
    Then that item pack load is SCHEMA
    Given an item moves rewrite labeled "p13-gear" with an unknown gear key
    Then that item pack load is SCHEMA

  Scenario: Missing item with a GM seated queues nothing
    Given the item port is locked
    And an item pack labeled "item-gm" where Rowan lacks knife
    Then the item mark row requires knife and defend names no item
    And the item sheet lists an empty inventory
    When an item player sits with a GM
    Then the item player stderr is empty
    And item humans is 1
    And item world_time is 1080
    And item tick_index is 0
    And item tick files are none
    When the player line "mark pc_nell" is item MISSING_ITEM
    Then item humans is 1
    And item world_time is 1080
    And item tick_index is 0
    And item Rowan is in docks_shop
    And item actor pc_nell has no flags
    And item tick files are none
    And the item Rowan sheet is unchanged
    And item GM status queue is empty at world_time 1080
    And the item session is closed

  Scenario: Missing item with no GM queues nothing
    Given the item port is locked
    And an item pack labeled "item-open" where Rowan lacks knife
    Then the item mark row requires knife and defend names no item
    And the item sheet lists an empty inventory
    When an item player sits with no GM
    Then the item player stderr is empty
    And item humans is 1
    And item world_time is 1080
    And item tick_index is 0
    And item tick files are none
    When the player line "mark pc_nell" is item MISSING_ITEM
    Then item humans is 1
    And item world_time is 1080
    And item tick_index is 0
    And item Rowan is in docks_shop
    And item actor pc_nell has no flags
    And item tick files are none
    And the item Rowan sheet is unchanged
    And the item session is closed

  Scenario: Missing item held knife still runs the opposed row
    Given the item port is locked
    And an item pack labeled "item-held" where Rowan holds knife
    Then the item mark row requires knife and defend names no item
    And the item sheet lists knife
    When an item player sits with a GM
    Then item world_time is 1080
    And item tick files are none
    When the GM queues defend for pc_nell
    Then the defend intent has no code and a queued id
    And item world_time is 1080
    And item tick files are none
    When the player writes the item line "mark pc_nell"
    Then the item player gained queued and not check
    And the item player stderr is empty
    And item world_time is 1080
    And item tick files are none
    And the item GM queue holds pc_nell defend and pc_rowan mark at world_time 1080
    When the GM item-commits the turn
    Then the item commit has no code
    And the item player gained a failed check and not faces
    And the item player stderr is empty
    And the item GM checks are Nell success against Rowan
    And item humans is 1
    And item world_time is 1085
    And item tick_index is 1
    And item Rowan is in docks_shop
    And item actor pc_nell is in docks_shop
    And item tick "tick-0001.jsonl" world_time goes from 1080 to 1085
    And item tick "tick-0001.jsonl" mark intent is player pc_rowan targeting pc_nell
    And item tick "tick-0001.jsonl" defend intent is player pc_nell
    And item actor pc_nell flags are guarded and missed
    And item actor pc_rowan has no flags
    And item tick "tick-0001.jsonl" derives neither mark nor defend
    And the item Rowan sheet is unchanged
    And the item session is closed
