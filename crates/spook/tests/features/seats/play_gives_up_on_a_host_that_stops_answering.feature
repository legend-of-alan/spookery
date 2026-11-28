Feature: Play gives up on a host that stops answering

  Scenario Outline: Play exits with an error from a stub host that <behavior>
    Given a stub host that <behavior>
    When play picks rowan on that stub host
    Then the command exits 1
    And stderr shows "<error>"

    Examples:
      | behavior                                         | error                             |
      | accepts the connection and says nothing          | Opening the connection timed out. |
      | completes the handshake and sends no hello        | Opening the connection timed out. |
      | sends the hello and only chatter after the claim | Claiming the character timed out. |
      | answers the claim and then says nothing          | Releasing the character timed out. |
