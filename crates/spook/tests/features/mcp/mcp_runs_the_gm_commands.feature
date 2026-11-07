Feature: /mcp runs the GM commands

  Scenario: A GM on /mcp and a GM on WebSocket see the same messages
    Given an AI GM on /mcp and a plain GM client on WebSocket
    When each sends travel for an unheld npc and commits
    Then both see the same messages and the same refusals
