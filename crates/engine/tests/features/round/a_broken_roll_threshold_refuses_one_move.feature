Feature: A broken roll threshold refuses one move, not the round

  Scenario: A roll threshold that fails to evaluate refuses that move and the rest of the round lands
    Given a docks pack whose "pick" rule rolls grit against a threshold that divides by zero
    When Rowan walks to the street and Wren plays pick in the same round
    Then Wren's pick is refused OUT_OF_RANGE and Rowan is on the street
    And the next round plays and spook verify passes
