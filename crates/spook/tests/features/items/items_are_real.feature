Feature: Items are real

  Scenario: Tonic is used up
    Given harbormouse holds 1 tonic and has 3 of 10 hp
    When Harbormouse types "use tonic"
    Then harbormouse has 7 hp and 0 tonic
    And "use tonic" again is rejected with NOT_IN_SPEC by the engine

  Scenario: Take and drop
    Given docks_shop has 3 lamp_oil
    When rowan takes lamp_oil
    Then docks_shop has 2 and rowan has 1

  Scenario: Using an item with no use rule is refused, and the turn is kept
    Given harbormouse holds 1 latch at a seated table
    When harbormouse plays "use latch"
    Then harbormouse is refused with NOT_IN_SPEC
    And no round was spent and harbormouse still holds 1 latch

  Scenario Outline: A line naming nothing the player can act on reads like an unknown word
    Given a docks table
    When Harbormouse types "<line>"
    Then the line is rejected with NOT_IN_SPEC and no round was written

    Examples:
      | line               |
      | strike bram        |
      | strike nobody      |
      | wield chock        |
      | wield zzz          |
      | take latch         |
      | take lamp_oil      |
      | use banana         |
      | drink              |
      | strike harbormouse |

  Scenario: Dropping an item you do not hold is refused
    Given docks_shop has 3 lamp_oil
    When rowan drops lamp_oil
    Then the round names MISSING_ITEM and rowan still has no lamp_oil

  Scenario: Wield a weapon
    Given harbormouse holds a poisoned_dagger tagged weapon
    When Harbormouse types "wield poisoned_dagger"
    Then harbormouse's wielded slot is poisoned_dagger
    And the GM summary lists "actor_wielded_item harbormouse"

  Scenario: Only weapons can be wielded
    Given a docks table
    When Harbormouse types "wield tonic"
    Then it is rejected with NOT_A_WEAPON and the slot is unchanged

  Scenario: Attack uses the wielded weapon's rule
    Given harbormouse wields poisoned_dagger
    When harbormouse attacks wren
    Then wren's hp is 5

  Scenario: Bare hands fall back
    Given harbormouse wields nothing
    When harbormouse attacks wren
    Then wren's hp is 7

  Scenario: A wielded item's rule wins for any verb
    Given harbormouse wields a poisoned_dagger with a strike rule
    When Harbormouse types "strike wren"
    Then wren's hp is 4

  Scenario: Dropping the wielded item empties the slot
    Given harbormouse wields poisoned_dagger
    When harbormouse drops poisoned_dagger
    Then harbormouse's wielded slot is empty in the same round

  Scenario: A save is an outcome, not a refusal
    Given wren saves against the poison with a seeded Constitution roll of 15 vs 12
    When harbormouse hits a seated wren with poisoned_dagger
    Then wren loses 3 hp, is not poisoned, sees "The poison burns, then fades.", and the round shows no refusal
    And wren is told she saved
    And spook verify reproduces both rolls
