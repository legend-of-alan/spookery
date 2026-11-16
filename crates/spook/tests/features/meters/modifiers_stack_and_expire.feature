Feature: Modifiers stack per application, add ticks on reapply, and expire at once

  Background:
    Given the docks pack with one-tick rounds and rowan's vigor at 20, with three PCs in the shop

  Scenario: Two moves apply fatigue with different strength, and both count
    Given "fatigue" modifies vigor and has max_stacks 4
    And the jab verb applies fatigue -1 and the slam verb applies fatigue -2, each for 10 ticks
    When rowan is hit by the jab and then the slam in one round
    Then rowan has 2 stacks of fatigue and vigor is 3 lower
    And the GM sees "actor_modifier rowan fatigue" as a changed key
    And the GM sees "actor_meter rowan vigor" as a changed key
    And spook verify reproduces the journal

  Scenario: At the cap a stronger fatigue replaces the weakest stack
    Given "fatigue" modifies vigor and has max_stacks 4
    And rowan has 4 stacks of fatigue of -2, -1, -2 and -2, with 2 ticks left
    When the slam applies fatigue -3 for 10 ticks
    Then rowan has 4 stacks of fatigue of -2, -3, -2 and -2
    And vigor is 9 lower, not 7

  Scenario: At the cap an equal fatigue changes no stack and still adds its ticks
    Given "fatigue" modifies vigor and has max_stacks 4
    And rowan has 4 stacks of fatigue of -2 each, with 2 ticks left
    When the jab applies fatigue -2 for 10 ticks
    Then rowan still has 4 stacks of -2 and vigor is 8 lower
    And the fatigue has 10 ticks left, not 12, since 10 is the longest fatigue so far

  Scenario: A reapplied fatigue adds its ticks, up to the longest fatigue so far
    Given "fatigue" modifies vigor and has no max_stacks
    And the jab verb applies fatigue -1 for 3 ticks
    And the slam verb applies fatigue -1 for 2 ticks
    When rowan is hit by the jab and, 1 tick later, by the slam
    Then the fatigue has 3 ticks left, not 4, since 3 is the longest fatigue so far

  Scenario: A longer fatigue raises the cap to its own ticks
    Given "fatigue" modifies vigor and has no max_stacks
    And the jab verb applies fatigue -1 for 3 ticks
    And the slam verb applies fatigue -1 for 4 ticks
    When rowan is hit by the jab and, 1 tick later, by the slam
    Then the fatigue has 4 ticks left, not 6, since 4 is now the longest fatigue so far

  Scenario: A short fatigue adds its ticks to the ticks left
    Given "fatigue" modifies vigor and has no max_stacks
    And the jab verb applies fatigue -1 for 3 ticks
    And the slam verb applies fatigue -1 for 1 ticks
    When rowan is hit by the jab and, 2 ticks later, by the slam
    Then the fatigue has 2 ticks left, the 1 left plus its 1

  Scenario: A cost paid while fatigue is on moves vigor as its player reads it
    Given "fatigue" modifies vigor, and rowan holds one stack of fatigue of -2 over a stored vigor of 5
    When rowan makes a move that costs 1 vigor
    Then the GM sees "actor_meter rowan vigor" as a changed key, going from 3 to 2
    And a change rule that waits for vigor below 3 fires

  Scenario: Fatigue expires and all its stacks go at once
    Given "fatigue" modifies vigor and has max_stacks 4
    And rowan has 3 stacks of fatigue
    When the fatigue's 10 ticks pass with no reapply
    Then rowan has no fatigue and vigor is back
    And the GM sees "actor_modifier rowan fatigue" as a changed key
    And the GM sees "actor_meter rowan vigor" as a changed key
    And spook verify reproduces the journal

  Scenario: A modifier with no max_stacks never goes past one stack
    Given "soaked" modifies vigor and has no max_stacks
    When rowan is soaked by -1 and then soaked again by -1
    Then rowan has 1 stack of soaked and vigor is 1 lower
    And soaked has 10 ticks left after the second soaking

  Scenario: Different modifiers add up
    Given "rain" modifies sight by -1 and "fog" modifies sight by -2
    When rowan stands in rain and fog
    Then sight is 3 lower

  Scenario: A player sees only their own modifiers, and not a hidden one
    Given "fatigue" modifies vigor and has max_stacks 4
    And "dread" is a hidden modifier of vigor
    When wren tires rowan and fills them with dread
    Then rowan's change line shows fatigue and not dread
    And wren's change line shows neither

  Scenario: The pack check refuses max_stacks 0, ticks 0 and a modifier of no meter
    Given "fatigue" modifies vigor and has max_stacks 0
    And "chill" modifies warmth, which is not a meter
    And "numb" modifies vigor and a verb applies it for 0 ticks
    When the author runs spook check
    Then it refuses max_stacks 0, ticks 0 and the warmth meter

  Scenario: The pack check refuses a modifier listed twice at its second row
    Given "fatigue" modifies vigor and has max_stacks 4
    And "fatigue" modifies vigor and has no max_stacks
    When the author runs spook check
    Then it refuses fatigue as listed twice at line 6

  Scenario: The pack check refuses a verb that applies an undefined modifier
    Given a verb applies "scorch", which is not a modifier
    When the author runs spook check
    Then it refuses the scorch verb, saying scorch is not a modifier

  Scenario: Migrate drops a modifier whose def is gone or names another meter
    Given "fatigue" modifies vigor and has max_stacks 4
    And "soaked" modifies vigor and has no max_stacks
    And rowan has 3 stacks of fatigue and 1 of soaked
    When the pack drops fatigue and soaked now modifies rest
    And spook check reports rowan holds fatigue
    And the operator migrates the pack
    Then rowan has neither fatigue nor soaked
    And spook verify reproduces the journal

  Scenario: Migrate drops a meter the pack no longer gives
    Given rowan carries meter "luck" and the world carries meter "tide"
    When the pack drops luck and tide
    And spook check reports the world lists meter tide
    And the operator migrates the pack
    Then rowan has no luck and the world has no tide
    And spook verify reproduces the journal
