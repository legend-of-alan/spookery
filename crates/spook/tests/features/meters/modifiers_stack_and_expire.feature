Feature: Modifiers stack per application, refresh on reapply, and expire at once

  Background:
    Given the docks pack with one-minute rounds and pc_rowan's grit at 20, with three PCs in the shop

  Scenario: Two moves apply burn with different strength, and both count
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And the torch verb applies burn -1 and the brand verb applies burn -2
    When pc_rowan is hit by the torch and then the brand in one round
    Then pc_rowan has 2 stacks of burn and grit is 3 lower
    And the GM sees "modifier pc_rowan burn" as a changed key
    And the GM sees "actor_meter pc_rowan grit" as a changed key
    And spook verify reproduces the journal

  Scenario: At the cap a stronger burn replaces the weakest stack
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And pc_rowan has 4 stacks of burn of -2, -1, -2 and -2, with 2 minutes left
    When the brand applies burn -3
    Then pc_rowan has 4 stacks of burn of -2, -3, -2 and -2
    And grit is 9 lower, not 7
    And the burn ends 10 minutes from now, not 2

  Scenario: At the cap a weaker burn only refreshes
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And pc_rowan has 4 stacks of burn of -2 each, with 2 minutes left
    When the torch applies burn -1
    Then pc_rowan still has 4 stacks of -2 and grit is 8 lower
    And the burn ends 10 minutes from now, not 2

  Scenario: Burn expires and all its stacks go at once
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And pc_rowan has 3 stacks of burn
    When the burn's 10 minutes pass with no reapply
    Then pc_rowan has no burn and grit is back
    And the GM sees "modifier pc_rowan burn" as a changed key
    And the GM sees "actor_meter pc_rowan grit" as a changed key
    And spook verify reproduces the journal

  Scenario: A modifier with no max_stacks never goes past one stack
    Given "soaked" modifies grit, lasts 10 minutes, and has no max_stacks
    When pc_rowan is soaked by -1 and then soaked again by -1
    Then pc_rowan has 1 stack of soaked and grit is 1 lower
    And soaked ends 10 minutes after the second soaking

  Scenario: Different modifiers add up
    Given "rain" modifies sight by -1 and "fog" modifies sight by -2
    When pc_rowan stands in rain and fog
    Then sight is 3 lower

  Scenario: A player sees only their own modifiers, and not a hidden one
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And "dread" is a hidden modifier of grit
    When pc_wren burns pc_rowan and fills them with dread
    Then pc_rowan's change line shows burn and not dread
    And pc_wren's change line shows neither

  Scenario: The pack check refuses max_stacks 0 and a modifier of no meter
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 0
    And "chill" modifies warmth, which is not a meter
    When the author runs spook check
    Then it refuses max_stacks 0 and the warmth meter

  Scenario: The pack check refuses a modifier listed twice at its second row
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And "burn" modifies grit, lasts 10 minutes, and has no max_stacks
    When the author runs spook check
    Then it refuses burn as listed twice at line 7

  Scenario: The pack check refuses a verb that applies an undefined modifier
    Given a verb applies "scorch", which is not a modifier
    When the author runs spook check
    Then it refuses the scorch verb, saying scorch is not a modifier

  Scenario: Migrate drops a modifier whose def is gone or names another meter
    Given "burn" modifies grit, lasts 10 minutes, and has max_stacks 4
    And "soaked" modifies grit, lasts 10 minutes, and has no max_stacks
    And pc_rowan has 3 stacks of burn and 1 of soaked
    When the pack drops burn and soaked now modifies rest
    And the operator migrates the pack
    Then pc_rowan has neither burn nor soaked
    And spook verify reproduces the journal
