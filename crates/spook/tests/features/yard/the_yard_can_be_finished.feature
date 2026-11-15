Feature: The yard can be finished

  Scenario: A scripted session pays the lane's ticks, reaches the ending, and spook verify passes
    When the yard script is played with the stub GM
    Then Harbormouse reaches the lane at the exit's cost in ticks and hears the ending
    And spook verify passes on the save

  Scenario: The engine refuses an AI write
    Given the stub GM attempts to set gold directly
    Then the attempt is refused and the refusal is in the log
