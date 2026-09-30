@v0
Feature: Shop lives
  Proof: v0_t03_shop_lives
  lamp_oil changes by at most stock_rate per tick and stays ≥ 0.

  Scenario: lamp_oil changes by at most stock_rate per tick and stays at least 0
    Given a temp copy of pack packs/oneshot-docks
    Then docks_shop lamp_oil stock_rate is 4
    And docks_shop lamp_oil is at least 0
    When 13 ticks record docks_shop lamp_oil
    Then every tick exits 0
    And each docks_shop lamp_oil change stays within stock_rate
    And each recorded docks_shop lamp_oil quantity is at least 0
