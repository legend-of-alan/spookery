Feature: One player seat and one GM seat

  Scenario: A second player leaves the seat unchanged
    Given a docks table with a seated player
    When a second player connects
    Then now.yaml is unchanged since the seat
    And humans is 1
    And the present actor is pc_rowan
    And state_rev is 1
    And tick index is 0
    And world_time is 1080
    And no tick file was written
    And the tape file is absent

  Scenario: A GM hello writes nothing
    Given a docks table with a seated GM
    Then now.yaml is unchanged since the table opened
    And humans is 0

  Scenario: A second GM leaves the table unchanged
    Given a docks table with a seated GM
    When a second GM connects
    Then now.yaml is unchanged since the table opened
    And humans is 0
