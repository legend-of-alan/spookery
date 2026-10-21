Feature: Items are real

  Scenario: Tonic is used up
    Given nell holds 1 tonic and has 3 of 10 hp
    When nell types "use tonic"
    Then nell has 7 hp and 0 tonic
    And "use tonic" again is rejected with MISSING_ITEM by the kernel

  Scenario: Take and drop
    Given docks_shop has 3 lamp_oil
    When rowan takes lamp_oil
    Then docks_shop has 2 and rowan has 1

  Scenario: Using an item with no use rule is refused, and the turn is kept
    Given lark holds 1 latch at a seated table
    When lark plays "use latch"
    Then lark is refused with NOT_IN_SPEC and the sentence "You can't use Iron latch."
    And no round was spent and lark still holds 1 latch

  Scenario: Using a cast member with no use rule is refused, and the turn is kept
    Given lark holds 1 latch at a seated table
    When lark plays "use npc_briar"
    Then lark is refused with NOT_IN_SPEC and the sentence "You can't use Briar."
    And no round was spent and lark still holds 1 latch

  Scenario: Using something unknown keeps the plain refusal
    Given lark holds 1 latch at a seated table
    When lark plays "use banana"
    Then lark is refused with NOT_IN_SPEC and no sentence
    And no round was spent and lark still holds 1 latch

  Scenario: A hidden exit reads the same as an unknown word
    Given lark holds 1 latch at a seated table with a hidden trapdoor
    When lark plays "use trapdoor"
    Then lark is refused with NOT_IN_SPEC and no sentence
    And no round was spent and lark still holds 1 latch

  Scenario: A revealed exit is named
    Given lark holds 1 latch at a seated table with a trapdoor lark knows about
    When lark plays "use trapdoor"
    Then lark is refused with NOT_IN_SPEC and the sentence "You can't use trapdoor."
    And no round was spent and lark still holds 1 latch

  Scenario: Someone elsewhere reads the same as an unknown word
    Given lark holds 1 latch at a seated table
    When lark plays "use npc_oden"
    Then lark is refused with NOT_IN_SPEC and no sentence
    And no round was spent and lark still holds 1 latch

  Scenario: The tonic has one verb, use
    Given nell holds 1 tonic and has 3 of 10 hp
    When nell types "drink"
    Then the line is rejected with NOT_IN_SPEC and nell still has 3 hp and 1 tonic

  Scenario: Taking an item that is not there is refused
    Given docks_shop has no lamp_oil
    When rowan takes lamp_oil
    Then the tick names MISSING_ITEM and the shop has no lamp_oil

  Scenario: Dropping an item you do not hold is refused
    Given docks_shop has 3 lamp_oil
    When rowan drops lamp_oil
    Then the tick names MISSING_ITEM and rowan still has no lamp_oil

  Scenario: Unknown item
    Given now.yaml lists an item with no definition
    Then spook check reports an unknown item diagnostic

  Scenario: Wield a weapon
    Given lark holds a poisoned_dagger tagged weapon
    When lark types "wield poisoned_dagger"
    Then lark's wielded slot is poisoned_dagger

  Scenario: Only weapons can be wielded
    When lark types "wield tonic"
    Then it is rejected with NOT_A_WEAPON and the slot is unchanged

  Scenario: Attack uses the wielded weapon's rule
    Given lark wields poisoned_dagger
    When lark attacks nell and hits
    Then item:poisoned_dagger/attack ran, not verb:attack

  Scenario: Bare hands fall back
    Given lark wields nothing
    When lark attacks nell
    Then verb:attack ran

  Scenario: Dropping the wielded item empties the slot
    Given lark wields poisoned_dagger
    When lark drops poisoned_dagger
    Then lark's wielded slot is empty in the same tick

  Scenario: Resist is an outcome, not a refusal
    Given nell resists the poison with a seeded Constitution roll of 15 vs 12
    When lark hits nell with poisoned_dagger
    Then nell loses 3 hp, is not poisoned, sees "The poison burns, then fades.", and the tick shows no refusal
    And spook verify reproduces both rolls
