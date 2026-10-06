Feature: npc_act does not call a model

  Scenario: npc_act leaves the save unchanged
    Given a docks table with a seated GM
    When the GM calls npc_act for mira
    Then now.yaml is unchanged since the table opened
