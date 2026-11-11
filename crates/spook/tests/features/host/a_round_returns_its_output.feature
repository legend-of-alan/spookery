Feature: A round returns its output

  Scenario: A GM commit that fast-forwards still reports the committed round's rolls
    Given a docks table with a GM and a travelling wren seated
    When the GM queues "grit" for "rowan"
    And the GM queues "hit" for "rowan"
    And the GM queues a 10-tick travel for "rowan"
    And the GM commits the round
    Then the commit reply lists one check for "rowan"
    And the commit reply lists the committed round's dice line

  Scenario: A busy refusal with no GM refreshes every seated player
    Given a docks table with rowan and wren seated and no GM
    And both players are travelling
    When "rowan" sends a move
    Then "rowan" gets a refused line with code BUSY
    And each seated player then gets a resume line and a perceive line
    And the host prints its status once

  Scenario: A commit with no fast-forward reports the same as before
    Given a docks table with a GM and wren seated
    When the GM queues "grit" for "rowan"
    And the GM commits the round
    Then the commit reply lists one check for "rowan"
