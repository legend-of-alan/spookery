Feature: An intent the round has no room for is refused BUDGET

  Scenario: An intent past the round's intent limit is refused BUDGET
    Given the docks pack allows 1 intent per round
    When Harbormouse waits and Wren waits in the same round
    Then Wren's wait is refused BUDGET and nothing is left queued

  Scenario: A rule over the state-key limit and every intent after it are refused BUDGET
    Given the docks pack allows 1 state key per round and a row "overset" that sets 2 flags
    When Rowan plays overset and then Wren waits in the same round
    Then Rowan's overset and Wren's wait are both refused BUDGET and nothing is left queued

  Scenario Outline: A modifier spends its own key and its meter's, so brace and its two act counts need four
    Given the docks pack allows <keys> state keys per round and a row "brace" that adds 2 to vigor through a modifier
    When Rowan plays brace
    Then Rowan's <outcome>

    Examples:
      | keys | outcome                                                                                             |
      | 3    | verb is refused BUDGET                                                                              |
      | 4    | brace lands, and the GM sees both "actor_modifier rowan brace" and "actor_meter rowan vigor" change |

  Scenario Outline: A modifier that overflows its meter is refused BUDGET only when the meter's key does not fit
    Given the docks pack allows <keys> state keys per round and a row "brace" that adds 9223372036854775807 to vigor through a modifier
    When Rowan plays brace
    Then Rowan's verb is refused <code>

    Examples:
      | keys | code         |
      | 2    | BUDGET       |
      | 3    | OUT_OF_RANGE |
