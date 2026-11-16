Feature: The lane ends the story

  Scenario: A scripted session pays the lane's ticks, reaches the ending, and spook verify passes
    When the lane script is played with the stub GM
    Then Harbormouse reaches the lane at the exit's cost in ticks and hears the ending
    And spook verify passes
