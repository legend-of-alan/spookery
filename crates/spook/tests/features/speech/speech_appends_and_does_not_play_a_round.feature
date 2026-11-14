Feature: Speech appends to the tape and does not play a round

  Scenario: A seated player says a line to the table
    Given a docks table with a seated player
    When the player says "hello"
    Then the tape contains the text "hello"
    And the tape address is "table"
    And save.yaml is unchanged since the seat
    And state_rev is unchanged
    And no round record was written

  Scenario: A seated player says a line to the room
    Given a docks table with a seated player
    When the player says "the lamp is lit" to the place
    Then the tape contains the text "the lamp is lit"
    And the tape address is "place"
    And the tape names the place "docks_shop"
    And the tape recipients are "rowan"
    And save.yaml is unchanged since the seat
    And no round record was written

  Scenario: A seated player says a line to aldric
    Given a docks table with a seated player
    When the player says "good evening" to aldric
    Then the tape contains the text "good evening"
    And the tape address is "actor"
    And the tape names the actor "aldric"
    And save.yaml is unchanged since the seat
    And no round record was written

  Scenario: A sentence that does not name an actor stays at the table
    Given a docks table with a seated player
    When the player says "good evening mira"
    Then the tape contains the text "good evening mira"
    And the tape address is "table"
    And save.yaml is unchanged since the seat
    And no round record was written

  Scenario: Say to aldric while nobody is seated
    Given a docks table
    When say to aldric is submitted on an empty table saying "good evening"
    Then the tape contains the text "good evening"
    And the tape address is "actor"
    And the tape names the actor "aldric"
    And save.yaml matches the bytes from before the line
    And no round record was written

  Scenario: A rejoining player gets the newest speech
    Given a docks table with "rowan" seated
    And the GM has said 300 numbered lines to "rowan"
    When "rowan" leaves and sits again
    Then the replayed say lines are "line44" through "line299", in order
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
    Then the newest replayed say line is "still here"

  Scenario: The GM says a line as Aldric
    Given a docks table with "rowan" seated
    When the GM says "Mind the counter." to the place "docks_shop" as "aldric"
    Then "rowan" reads "Aldric: Mind the counter." under the GM's seq
    And the tape line is from "npc:aldric"
    And the tape recipients are "rowan"
    When "rowan" leaves and sits again
    Then the newest replayed say line is "Mind the counter."

  Scenario Outline: The GM can speak only as an NPC where that NPC stands
    Given a docks table with "rowan" seated
    When the GM says "hello" to the place "docks_shop" as "<speaker>"
    Then that say is refused SCHEMA and nothing was written

    Examples:
      | speaker    |
      | rowan      |
      | nobody     |
      | gm         |
      | npc:aldric |
      | mira       |

  Scenario: A player can't speak as an NPC
    Given a docks table with "rowan" seated
    When "rowan" says "hello" to the table as "aldric"
    Then that say is refused SCHEMA and nothing was written

  Scenario: A player's place say must be where they stand
    Given a docks table with "rowan" seated
    When "rowan" says "hello" to the place "mira_rooms"
    Then that say is refused SCHEMA and nothing was written
