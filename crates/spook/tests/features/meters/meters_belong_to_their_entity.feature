Feature: Meters belong to their entity

  Scenario: Mira's shop day is a meter only Mira carries
    Given cast/mira.md declares a "shop_day" meter
    When the pack loads
    Then Mira has a shop_day meter
    And Bram and rowan do not
    When a round runs and Mira's habit reads and resets shop_day
    Then the habit runs as it did when shop_day was a clock
    And the GM sees shop_day on Mira's meters
    And rowan's view does not list it

  Scenario: The pack check refuses a rule that writes a scoped meter on the wrong actor
    Given cast/mira.md declares a "shop_day" meter
    And Bram has a habit that adds 1 to his own shop_day
    When the author runs spook check
    Then it refuses Bram's habit, saying only Mira carries shop_day

  Scenario: A scoped meter id that is already global is refused
    Given rules/meters.yaml defines "grit"
    And cast/mira.md also declares a "grit" meter
    When the author runs spook check
    Then it refuses the second grit

  Scenario: A player in the same room does not see another actor's meter change
    Given rowan and Mira stand in the shop
    When Mira's habit adds 1 to her shop_day
    Then Mira's change line shows shop_day
    And rowan's change line does not

  Scenario: A player does not see a flag change
    Given rowan stands in the shop
    When a rule sets the world flag "bells_rung"
    And a rule sets Mira's flag "shop_open"
    Then rowan's change line shows neither flag
    And the GM sees both as changed keys

  Scenario: Two stories share one dawn countdown
    Given world/meters.yaml declares a "dawn" meter at 6
    And story/siege.yaml has a rule that adds -1 to meter dawn of "world"
    And story/vigil.yaml has a rule that adds -1 to meter dawn of "world"
    And story/vigil.yaml has a change rule that watches dawn cross "<=0"
    When both rules run in one round, three rounds in a row
    Then dawn is 0, held once by the world
    And no actor carries a dawn meter
    And the vigil's change rule fires once

  Scenario: A rule naming a meter its entity does not carry is refused
    Given world/meters.yaml declares a "dawn" meter
    And Mira carries no dawn meter
    And Mira has a habit that adds 1 to meter dawn of "self"
    When the author runs spook check
    Then it refuses Mira's habit, saying Mira carries no dawn meter

  Scenario: The world and an actor may each have a meter with the same name
    Given world/meters.yaml declares a "dawn" meter at 6
    And cast/mira.md declares her own "dawn" meter at 0
    And Mira has a habit that adds 1 to meter dawn of "self"
    When one round runs
    Then Mira's dawn is 1
    And the world's dawn is still 6

  Scenario: A world meter is hidden unless the pack shows it
    Given world/meters.yaml declares "dawn" with hidden false and "doom" with no hidden key
    When a rule adds -1 to meter dawn of "world" and to meter doom of "world"
    Then rowan's change line shows dawn and not doom
    And the GM sees both

  Scenario: A reconnecting player gets current world meter values
    Given world/meters.yaml declares "dawn" with hidden false and "doom" with no hidden key
    And dawn has fallen to 3 while rowan's player was away
    When the player reconnects and claims rowan
    Then the catch-up view lists dawn at 3
    And it does not list doom
