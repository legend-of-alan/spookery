Feature: The map shows exits as the actor last saw them

  Scenario: A hidden exit revealed elsewhere shows after a revisit
    Given the shop has an exit revealed by the found_trapdoor flag
    And Mira has visited the shop and left for the street
    When Mira finds the trapdoor on the street
    Then Mira's map shows the shop without the trapdoor
    When Mira walks back to the shop
    Then Mira's map shows the shop with the trapdoor

  Scenario: Finding the trapdoor while present shows it right away
    Given the shop has an exit revealed by the found_trapdoor flag
    And Mira is standing in the shop
    When Mira finds the trapdoor
    Then Mira's map shows the shop with the trapdoor after that round
