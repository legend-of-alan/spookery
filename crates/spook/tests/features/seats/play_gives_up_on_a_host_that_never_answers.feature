Feature: Play gives up on a host that never answers

  Scenario Outline: Play exits with an error from a stand-in host that <behaviour>
    Given a stand-in host that <behaviour>
    When play picks rowan on that stand-in host
    Then the command exits 1
    And stderr shows "<error>"

    Examples:
      | behaviour                                        | error                             |
      | accepts the connection and says nothing          | Opening the connection timed out. |
      | finishes the handshake and sends no hello        | Opening the connection timed out. |
      | sends the hello and only chatter after the claim | Claiming the character timed out. |
