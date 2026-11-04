Feature: Coding agents reach the GM seat

  Scenario: A coding agent calls the seat tools over loopback HTTP
    Given a docks table
    When a coding agent calls the GM tools over loopback HTTP
    Then world time moved forward

  Scenario: An open HTTP address is refused before the pack opens
    Given a docks table
    When the operator runs with an open MCP HTTP address
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: A public Origin does not take the GM seat
    Given a docks table
    When an HTTP request arrives with a public Origin
    Then the response is forbidden
    And a GM hello still sits

  Scenario: A second HTTP GM is told the table is full
    Given a docks table
    When a GM holds the seat over HTTP
    And another GM tries to sit over HTTP
    Then the newcomer is told the table is full
    When the GM over HTTP leaves
    Then a GM can sit over HTTP

  Scenario: The tools are the GM words
    Given a docks table
    When a GM holds the seat over HTTP
    And the GM lists the tools
    Then the tools are the GM words, and each input schema names these fields
      | tool     | fields                                                | required           |
      | status   |                                                       |                    |
      | perceive | as max_body_chars                                     | as                 |
      | queue    | id round_index actor_id move target to rationale_code | actor_id move      |
      | say      | address text                                          | address text       |
      | say      | address place text                                    | address place text |
      | say      | address actor text                                    | address actor text |
      | commit   | include_round session_id                              | include_round      |
      | release  |                                                       |                    |
    And a queue call with "actor_id", "move" and "to" is accepted

  Scenario: spook gm and a coding agent share one seat
    Given a docks table
    When a human GM sits on the game port
    And a coding agent tries to sit at that MCP endpoint
    Then the agent is told the table is full
    When the human GM leaves
    Then a coding agent can sit at that MCP endpoint

  Scenario: spook run serves /mcp without a flag
    Given a docks table
    When spook run listens with no MCP flag
    Then the reported MCP address serves /mcp

  Scenario: A request with an unknown MCP-Protocol-Version is refused
    Given a docks table
    When an HTTP request arrives with an unknown MCP-Protocol-Version
    Then the response status is 400

  Scenario: A request with the negotiated MCP-Protocol-Version is accepted
    Given a docks table
    When an HTTP request arrives with the negotiated MCP-Protocol-Version
    Then the response is accepted

  Scenario Outline: A Host other than 127.0.0.1 does not take the GM seat
    Given a docks table
    When an HTTP request arrives with Host <host>
    Then the response is forbidden
    And a GM hello still sits

    Examples:
      | host         |
      | localhost    |
      | evil.example |

  Scenario: A Host of 127.0.0.1 is accepted
    Given a docks table
    When an HTTP request arrives with Host 127.0.0.1
    Then the response is accepted

  Scenario: An HTTP agent delivers a say to a seated player
    Given a docks table with a seated player and an open MCP endpoint
    When a coding agent says good evening over HTTP
    Then the player saw a say from the host
