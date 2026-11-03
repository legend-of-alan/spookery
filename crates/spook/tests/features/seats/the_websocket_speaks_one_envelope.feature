Feature: The WebSocket door speaks one message envelope

  Scenario: A word the table does not know is refused in a numbered reply
    Given a docks table
    When a player connects over WebSocket
    Then the hello lists the version and the words travel, say, look, claim, release
    When it sends {"id":"c1","body":{"word":"fly"}}
    Then it gets {"seq":1,"reply_to":"c1","body":{"kind":"refused","code":"SCHEMA"}}

  Scenario: A GM drives a session over WebSocket
    Given a docks table with a seated player
    When a GM sits on the WebSocket door
    And the player waits
    Then the WebSocket GM hears the wait queued under the player's reply seq
    When the WebSocket GM commits
    Then the WebSocket GM gets the round and then the commit reply
    And the player hears that round under the same seq
    When the WebSocket GM says "good evening"
    Then the WebSocket GM's reply is its own say
    And the player hears the GM say "good evening"
    When the WebSocket GM releases the seat
    Then the WebSocket GM is told the seat is free
    And every message the WebSocket GM got after the hello has a larger seq
