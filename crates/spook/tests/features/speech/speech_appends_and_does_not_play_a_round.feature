Feature: Speech appends to the tape and does not play a round

  Scenario Outline: A seated player's line goes to the actor it is said to, or else to the table
    Given a docks table with a seated player
    When the player says <line>
    Then the tape contains the text "<text>"
    And the tape line is said to <audience>
    And save.yaml is unchanged since the seat
    And state_rev is unchanged
    And no round was played

    Examples:
      | line                     | text              | audience           |
      | "hello"                  | hello             | the table          |
      | "good evening" to aldric | good evening      | the actor "aldric" |
      | "good evening mira"      | good evening mira | the table          |

  Scenario: A seated player says a line to the room
    Given a docks table with a seated player
    When the player says "the lamp is lit" to the place
    Then the tape contains the text "the lamp is lit"
    And the tape line is said to the place "docks_shop"
    And the tape recipients are "rowan"
    And save.yaml is unchanged since the seat
    And no round was played

  Scenario: Say to aldric while nobody is seated
    Given a docks table
    When say to aldric is submitted on an empty table saying "good evening"
    Then the tape contains the text "good evening"
    And the tape line is said to the actor "aldric"
    And save.yaml matches the bytes from before the line
    And no round was played

  Scenario: A rejoining player gets the newest speech
    Given a docks table with "rowan" seated
    And the GM has said 300 numbered lines to "rowan"
    When "rowan" leaves and sits again
    Then the replayed lines are "line44" through "line299", in order
    And the replayed lines ride in the claim's reply

  Scenario: Speech for someone else doesn't use up the replay window
    Given a docks table with "rowan" and "wren" seated
    And the GM has said 300 lines to "wren" and then 3 lines to "rowan"
    When "rowan" leaves and sits again
    Then "rowan" gets exactly those 3 lines, in order

  Scenario: A long chatty world still accepts speech
    Given a docks table with "rowan" seated
    And the tape already holds a full 1 MiB of old speech
    When the player says "still here"
    And "rowan" leaves and sits again
    Then the newest replayed line is "still here"

  Scenario: The GM says a line as Aldric
    Given a docks table with "rowan" seated
    When the GM says "Mind the counter." to the place "docks_shop" as "aldric"
    Then "rowan" reads "Aldric: Mind the counter." under the GM's seq
    And the tape line is from "npc:aldric"
    And the tape recipients are "rowan"

  Scenario Outline: The GM can't speak as a PC, a missing actor or an NPC somewhere else
    Given a docks table with "rowan" seated
    When the GM says "hello" to the place "docks_shop" as "<speaker>"
    Then that say is refused SCHEMA and nothing was written

    Examples:
      | speaker |
      | rowan   |
      | nobody  |
      | mira    |

  Scenario: A player can't speak as an NPC
    Given a docks table with "rowan" seated
    When "rowan" says "hello" to the table as "aldric"
    Then that say is refused SCHEMA and nothing was written
