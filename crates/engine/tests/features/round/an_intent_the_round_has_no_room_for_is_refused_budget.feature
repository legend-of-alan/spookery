Feature: An intent the round has no room for is refused BUDGET

  Scenario: A move past the round's intent limit is refused BUDGET
    Given the d20 pack allows 1 intent per round
    When Lark waits and Briar waits in the same round
    Then Briar's wait is refused BUDGET and nothing is left queued

  Scenario: A rule over the state-key limit and every move after it are refused BUDGET
    Given the docks pack allows 1 state key per round and a row "overset" that sets 2 flags
    When Rowan plays overset and then Wren waits in the same round
    Then Rowan's overset and Wren's wait are both refused BUDGET and nothing is left queued

  Scenario Outline: A modifier spends its own key and its meter's, so brace and its two act counts need four
    Given the docks pack allows <keys> state keys per round and a row "brace" that adds 2 to grit through a modifier
    When Rowan plays brace
    Then Rowan's brace <outcome>

    Examples:
      | keys | outcome                                                                                      |
      | 3    | is refused BUDGET                                                                            |
      | 4    | lands, and the GM sees both "modifier rowan brace" and "actor_meter rowan grit" change |

  Scenario Outline: A modifier that overflows its meter is refused BUDGET only when the meter's key does not fit
    Given the docks pack allows <keys> state keys per round and a row "brace" that adds 9223372036854775807 to grit through a modifier
    When Rowan plays brace
    Then Rowan's brace is refused <code>

    Examples:
      | keys | code         |
      | 2    | BUDGET       |
      | 3    | OUT_OF_RANGE |
