Feature: Game time counts in ticks

  Scenario Outline: A day_tick window reads the tick within the pack's day
    Given briar restocks on day_tick <from> to <to> and ticks_per_day is <day>
    When an empty round lands at tick <tick> with briar in the shop
    Then docks_shop holds <oil> lamp_oil

    Examples:
      | day   | from | to  | tick | oil |
      | 144   | 0    | 36  | 160  | 8   |
      | 144   | 0    | 36  | 180  | 4   |
      | 144   | 120  | 24  | 130  | 8   |
      | 144   | 120  | 24  | 154  | 8   |
      | unset | 480  | 960 | 1920 | 8   |
      | unset | 480  | 960 | 2400 | 4   |

  Scenario Outline: Only a round that crosses the end of the day resets each actor's acts for the day
    Given every actor has used its acts for the day at tick <start> and ticks_per_day is <day>
    When a seated round runs
    Then the round ends at tick <end> and every actor's acts for the day are <acts>

    Examples:
      | day   | start | end  | acts |
      | 144   | 140   | 145  | 0    |
      | unset | 140   | 145  | 3    |
      | unset | 1436  | 1441 | 0    |
