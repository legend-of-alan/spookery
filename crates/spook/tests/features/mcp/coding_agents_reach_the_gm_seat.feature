Feature: Coding agents reach the GM seat

  Scenario: A coding agent calls the seat tools over stdio
    Given a docks table
    When a coding agent calls the GM tools over the stdio bridge
    Then world time moved forward

  Scenario: A coding agent calls the seat tools over loopback HTTP
    Given a docks table
    When a coding agent calls the GM tools over loopback HTTP
    Then world time moved forward

  Scenario: A second bridge is told the table is full
    Given a docks table
    When the GM seat is held and another agent opens the bridge
    Then the command exits 1
    And stdout is empty
    And stderr shows "The table is full. [SESSION_FULL]"

  Scenario: A second HTTP session is told the table is full
    Given a docks table
    When the GM seat is held and another agent opens HTTP
    Then the refusal names SESSION_FULL

  Scenario: An open HTTP address is refused before the pack opens
    Given a docks table
    When the operator runs with an open MCP HTTP address
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: An open bridge address is refused before a dial
    Given a docks table
    When the operator runs mcp bound to an open address
    Then the command exits 1
    And stderr is NOT_IN_SPEC
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: The bridge with nothing listening leaves the pack
    Given a docks table
    When the operator runs mcp with nothing listening
    Then the command exits 1
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: A public Origin does not take the GM seat
    Given a docks table
    When an HTTP request arrives with a public Origin
    Then the response is forbidden
    And a GM hello still sits

  Scenario Outline: A GM on one door keeps the seat from a GM on another
    Given a docks table
    When a GM holds the seat over <holder>
    And another GM tries to sit over <newcomer>
    Then the newcomer is told the table is full
    When the GM over <holder> leaves
    Then a GM can sit over <newcomer>

    Examples:
      | holder     | newcomer   |
      | HTTP       | TCP        |
      | HTTP       | the bridge |
      | TCP        | HTTP       |
      | the bridge | HTTP       |
      | TCP        | the bridge |
      | the bridge | TCP        |

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

  Scenario: Closing the HTTP session frees the GM seat
    Given a docks table
    When an HTTP GM session ends
    Then another GM can sit over HTTP

  Scenario: The bridge delivers a say to a seated player
    Given a docks table with a seated player
    When a coding agent says good evening over the bridge
    Then the player saw a say from the host
