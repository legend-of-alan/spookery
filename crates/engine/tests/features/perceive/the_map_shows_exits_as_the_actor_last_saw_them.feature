Feature: The map shows exits as the actor last saw them

  Scenario: A hidden exit revealed elsewhere shows after a revisit
    Given the shop has a hidden exit revealed by fact_trapdoor
    And Mira has visited the shop and left for the street
    When Mira learns fact_trapdoor on the street
    Then Mira's map shows the shop without the trapdoor
    When Mira walks back to the shop
    Then Mira's map shows the shop with the trapdoor

  Scenario: A reveal while present shows right away
    Given the shop has a hidden exit revealed by fact_trapdoor
    And Mira is standing in the shop
    When Mira learns fact_trapdoor
    Then Mira's map shows the shop with the trapdoor after that round
