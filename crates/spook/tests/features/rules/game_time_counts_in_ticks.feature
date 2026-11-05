Feature: The engine counts only ticks

  Scenario Outline: A cycle window reads the tick within the named cycle
    Given rules/budget.yaml sets cycle <cycle> to <length>
    And briar restocks on cycle <cycle> from <from> to <to>
    When an empty round lands at tick <tick> with briar in the shop
    Then docks_shop holds <oil> lamp_oil

    Examples:
      | cycle | length | from | to  | tick | oil |
      | day   | 1440   | 480  | 960 | 500  | 8   |
      | day   | 1440   | 480  | 960 | 1920 | 8   |
      | day   | 1440   | 480  | 960 | 2400 | 4   |
      | tide  | 144    | 0    | 36  | 160  | 8   |
      | tide  | 144    | 0    | 36  | 180  | 4   |
      | tide  | 144    | 120  | 24  | 130  | 8   |
      | tide  | 144    | 120  | 24  | 154  | 8   |

  Scenario: Mira restocks inside the docks day
    Given the docks pack, whose budget sets cycle day to 1440
    When an empty round lands at tick 500 with Mira in the shop
    Then docks_shop holds 8 lamp_oil

  Scenario Outline: Only a round that crosses the end of an idle window gives idle acts back
    Given idle_act_window_ticks is <window> and every actor has used its idle acts at tick <start>
    When a seated round runs
    Then the round ends at tick <end> and every actor's idle acts this window are <acts>

    Examples:
      | window | start | end  | acts |
      | 144    | 140   | 145  | 0    |
      | 1440   | 140   | 145  | 3    |
      | 1440   | 1436  | 1441 | 0    |

