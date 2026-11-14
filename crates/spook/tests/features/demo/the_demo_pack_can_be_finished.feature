Feature: The demo pack can be finished

  Scenario: The player who bests Briar sees only the gate ending, and spook verify passes on the save
    Given demo-d20 and Corin and the gate script
    When the demo script is played with the stub GM
    Then the player sees only the gate ending
    And spook verify passes on the save

  Scenario: The player who wards the wheel sees only the warded ending
    Given demo-d20 and Corin and the ward script
    When the demo script is played with the stub GM
    Then the player sees only the warded ending

  Scenario: The player who braces the wheel sees only the braced ending
    Given demo-d20 and Corin and the brace script
    When the demo script is played with the stub GM
    Then the player sees only the braced ending

  Scenario: The player whose hp reaches 0 sees only the knocked-out ending
    Given demo-d20 and Corin and the knocked-out script
    When the demo script is played with the stub GM
    Then the player sees only the knocked-out ending

  Scenario: The player still out at dusk sees only the dusk ending, and the cart is refused
    Given demo-d20 and Corin and the dusk script
    When the demo script is played with the stub GM
    Then the player sees only the dusk ending
    And the player's "travel cart" is refused with "The cart won't move."
