Feature: The showcase pack can be finished

  Scenario: A scripted session reaches an ending
    Given sample-d20 and a recorded line script
    When the script is played with the stub GM
    Then the ending flag is set within the session
    And spook verify and spook audit pass on the save

  Scenario: Every choice has a reachable ending
    Then each of the 2-3 branch scripts reaches an ending

  Scenario: A journey takes time and ends in a beat
    When the party travels through the locked exit after unlocking it
    Then they arrive at the exit's cost in minutes and the arrival beat fires

  Scenario: The consumable is used up
    When a player uses the consumable
    Then it leaves their inventory

  Scenario: The engine refuses an AI write
    Given the stub GM attempts to set gold directly
    Then the attempt is refused and the refusal is in the log
