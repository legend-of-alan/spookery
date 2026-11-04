Feature: Migrate moves clocks onto meters

  Scenario: A clock with one owner becomes a meter on that owner
    Given a docks save from before meters, with Mira's "shop_day" clock at 1 and pc_rowan's "assassination_debt" clock at 2
    When the author runs spook migrate
    Then migrate exits 0
    And Mira's shop_day meter is 1
    And pc_rowan's assassination_debt meter is 2

  Scenario: A clock with no owners becomes a world meter
    Given world/meters.yaml declares a "dawn" meter at 6
    And a docks save from before meters, with an unowned "dawn" clock at 3
    When the author runs spook migrate
    Then migrate exits 0
    And the world's dawn is 3

  Scenario: A clock with two owners is refused
    Given a docks save from before meters, with a "shop_day" clock owned by Mira and Bram
    When the author runs spook migrate
    Then migrate exits 1 saying "clock 'shop_day' has 2 owners, and a meter has one holder"
