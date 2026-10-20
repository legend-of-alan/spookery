Feature: A round returns its output

  Scenario: A GM commit that fast-forwards still reports the committed round's rolls
    Given a docks table with a GM and "pc_rowan" seated
    When the GM queues "grit" for "pc_rowan"
    And the GM queues a 10-minute travel for "pc_rowan"
    And the GM commits
    Then the commit reply lists one check for "pc_rowan"
    And the commit reply lists that check's dice line

  Scenario: A busy refusal with no GM refreshes every seated player
    Given a docks table with "pc_rowan" and "pc_wren" seated and no GM
    And both players are travelling
    When "pc_rowan" sends a move
    Then "pc_rowan" gets a refused line with code BUSY
    And each seated player then gets a resume line and a perceive line
    And the host prints its status once

  Scenario: A commit with no fast-forward reports the same as before
    Given a docks table with a GM and "pc_rowan" seated
    When the GM queues "grit" for "pc_rowan"
    And the GM commits
    Then the commit reply lists one check for "pc_rowan"
