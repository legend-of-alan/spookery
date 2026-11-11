Feature: The demo pack can be finished

  Scenario: The ending flag is set and spook verify passes on the save
    Given demo-d20 and Corin and a recorded line script
    When the demo script is played with the stub GM
    Then the ending flag is set within the session
    And spook verify passes on the save

  Scenario: The player sees the nail-kept ending and not the default ending
    Given demo-d20 and Corin and the brace script
    When the demo script is played with the stub GM
    Then the player sees the nail-kept ending and not the default ending
