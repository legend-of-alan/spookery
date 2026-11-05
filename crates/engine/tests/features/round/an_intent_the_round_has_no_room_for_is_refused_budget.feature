Feature: An intent the round has no room for is refused BUDGET

  Scenario: A move past the round's intent limit is refused BUDGET
    Given the d20 pack allows 1 intent per round
    When Lark waits and Briar waits in the same round
    Then Briar's wait is refused BUDGET and nothing is left queued

  Scenario: A rule over the state-key limit and every move after it are refused BUDGET
    Given the docks pack allows 1 state key per round and a row "overset" that sets 2 flags
    When Rowan plays overset and then Wren waits in the same round
    Then Rowan's overset and Wren's wait are both refused BUDGET and nothing is left queued
