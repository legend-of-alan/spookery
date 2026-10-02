@v02
Feature: Wire codec
  Proof: v02_p05_initialize_writes_no_tape_and_the_four_shapes_keep_key_order
  The GM socket does not receive tape bytes before MCP initialize. initialize does not append the tape. GM scrollback is session_status with since. Player scrollback on hello stays the v0.1 lines. A say tool result is still followed by its spook/event, not preceded by it. A player say still reaches the seated GM as a spook/event. spook gm --stdio, when nothing is listening, still exits 1. Stderr is the operating-system error. Stdout is empty. The four shapes keep key order.

  Scenario: Initialize writes no tape and the four shapes keep key order
    Given a wire-codec quiet docks pack and the port lock
    When wire-codec gm --stdio runs with nothing listening
    Then wire-codec gm --stdio exits 1
    And wire-codec gm --stdio stdout is empty
    And wire-codec gm --stdio stderr is the refused connect error
    And wire-codec gm --stdio stderr does not contain NOT_IN_SPEC
    And the wire-codec quiet pack bytes are unchanged
    Given a wire-codec docks pack with spook run listening and a seated player
    When the wire-codec player says "before the chair"
    Then the wire-codec player shows that v0.1 say line once
    And that wire-codec player say keeps keys kind, from, address, text
    And the wire-codec tape is one line for that say at the seated clock
    And that wire-codec tape line keeps keys seq, from, address, place, actor, recipients, text, tick_index, world_time
    And wire-codec world_time and state_rev are unchanged
    When a wire-codec GM socket sends initialize before any other method
    Then the wire-codec initialize result has id 1 and a result
    And the wire-codec initialize line has no tape bytes
    And the wire-codec tape bytes are still the bytes from before initialize
    And the wire-codec tape still has 1 line
    When the wire-codec GM sends notifications/initialized
    And the wire-codec GM calls session_status with since 0
    Then wire-codec session_status events are the scrollback say
    And that wire-codec scrollback object keeps keys kind, seq, from, address, place, actor, recipients, text
    And wire-codec session_status last_seq is 1 and events length is 1
    And the wire-codec tape bytes are still the bytes from before initialize
    When the wire-codec player says "across the lane"
    Then the wire-codec player shows that v0.1 say line once
    And that wire-codec player say keeps keys kind, from, address, text
    And the seated wire-codec GM receives that say as a spook/event
    And that wire-codec live event keeps keys jsonrpc, method, params, kind, seq, from, address, place, actor, recipients, text
    And that wire-codec live event has no id
    And the wire-codec tape ends with that say and has 2 lines
    When the wire-codec GM calls say addressed to the table with text "from the chair"
    Then the wire-codec say tool result precedes its spook/event
    And the wire-codec tool object is that chair say
    And that wire-codec tool object keeps keys kind, seq, from, address, place, actor, recipients, text
    And the wire-codec spook/event after the tool result is that chair frame
    And the wire-codec tape has 3 lines
    And the wire-codec shard stops
