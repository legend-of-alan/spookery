Feature: npc_act does not call a model

  Scenario: npc_act returns LLM_DISABLED
    Given a docks table with a seated GM
    When the GM calls npc_act for mira
    Then the tool result code is LLM_DISABLED
    And now.yaml is unchanged since the table opened
