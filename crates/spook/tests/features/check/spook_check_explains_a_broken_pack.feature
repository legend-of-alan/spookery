Feature: spook check explains a broken pack

  Scenario: A copy of the docks pack passes
    Given a docks table
    When the operator runs spook check
    Then the check exits 0
    And stdout says "no problems"
    And stderr is empty
    And the pack bytes are unchanged

  Scenario Outline: A broken copy names the fault
    Given a docks table
    When the copy is broken by "<fault>"
    And the operator runs spook check
    Then the check exits 1
    And the report names "<file>"
    And the report names "<code>"
    And the report says "<phrase>"
    And the report has a location for "<file>"
    And stdout is empty
    And the pack bytes are unchanged

    Examples:
      | fault                               | file                       | code              | phrase                                                   |
      | missing when                        | rules/verbs.yaml           | SCHEMA            | missing field 'when'                                     |
      | unknown field                       | rules/verbs.yaml           | SCHEMA            | unknown field                                            |
      | duplicate move                      | rules/verbs.yaml           | SCHEMA            | listed twice                                             |
      | unknown format tag                  | rules/verbs.yaml           | SCHEMA            | does not use                                             |
      | missing meters                      | rules/meters.yaml          | NOT_IN_SPEC       | missing                                                  |
      | meter min above base_cap            | rules/meters.yaml          | SCHEMA            | min above base_cap                                       |
      | meter base_cap above max            | rules/meters.yaml          | SCHEMA            | base_cap above max                                       |
      | meter base outside its range        | rules/meters.yaml          | SCHEMA            | base outside min and base_cap                            |
      | pack_rev mismatch                   | state/save.yaml            | PACK_REV_MISMATCH | pack_rev                                                 |
      | an actor the cast lacks             | state/save.yaml            | SCHEMA            | is not a cast member                                     |
      | a control type with two keys        | state/save.yaml            | SCHEMA            | two keys where one belongs                               |
      | an empty tags key                   | world/items/latch.md       | SCHEMA            | or write [] for none                                     |
      | an empty modifiers key              | state/save.yaml            | SCHEMA            | or write {} for none                                     |
      | exit to an unknown place            | world/map.yaml             | SCHEMA            | not a place                                              |
      | duplicate exit                      | world/map.yaml             | SCHEMA            | listed twice                                             |
      | exits in a place file               | world/places/docks_shop.md | SCHEMA            | unknown field 'exits'                                    |
      | a path with no arrow                | world/map.yaml             | SCHEMA            | -> or <->                                                |
      | missing map                         | world/map.yaml             | NOT_IN_SPEC       | missing                                                  |
      | a target alias                      | rules/verbs.yaml           | SCHEMA            | not a target kind                                        |
      | a verb named look                   | rules/verbs.yaml           | SCHEMA            | 'look' is a built-in command                             |
      | a verb named say                    | rules/verbs.yaml           | SCHEMA            | 'say' is a built-in command                              |
      | flags under idle                    | cast/pc-rowan.md           | SCHEMA            | unknown field                                            |
      | a cast role named kind              | cast/pc-rowan.md           | SCHEMA            | unknown field 'kind'                                     |
      | idle maybe                          | rules/budget.yaml          | SCHEMA            | not on or off                                            |
      | zero round interval                 | rules/budget.yaml          | SCHEMA            | round_every_real_seconds                                 |
      | zero idle ticks                     | rules/budget.yaml          | SCHEMA            | idle_ticks_per_round                                     |
      | zero idle window                    | rules/budget.yaml          | SCHEMA            | idle_act_window_ticks                                    |
      | a zero-tick cycle                   | rules/budget.yaml          | SCHEMA            | cycle day is 0                                           |
      | a cycle past its length             | cast/mira.md               | SCHEMA            | cycle bound is not below cycle day                       |
      | an unknown cycle                    | cast/mira.md               | SCHEMA            | no cycle tide                                            |
      | world as an actor                   | world/places/gate_yard.md  | SCHEMA            | 'world' is not an actor                                  |
      | target on a move with none          | rules/verbs.yaml           | SCHEMA            | in of:, but its target is none                           |
      | owner target on an item move        | rules/verbs.yaml           | SCHEMA            | in of:, but its target is item                           |
      | id: target with no target           | rules/verbs.yaml           | SCHEMA            | in id:, but its target is none                           |
      | a save by target with none          | rules/verbs.yaml           | SCHEMA            | in by:, but its target is none                           |
      | target on an exit                   | world/map.yaml             | SCHEMA            | but an exit has no target                                |
      | a meter on the wrong actor          | cast/bram.md               | SCHEMA            | only mira carries meter 'shop_day'                       |
      | a cast meter that is global         | cast/mira.md               | SCHEMA            | meter 'vigor' is already in rules/meters.yaml            |
      | a meter its actor lacks             | cast/mira.md               | SCHEMA            | mira carries no meter 'dawn'                             |
      | self in a world change rule         | story/tide.yaml            | SCHEMA            | rule 'mark' watches a world change, which has no self    |
      | a check in a world change rule      | story/tide.yaml            | SCHEMA            | rule 'mark' watches a world change, which has no self    |
      | a narrate with no to                | rules/verbs.yaml           | SCHEMA            | rule 'ring' has a narrate with no to                     |
      | a named narrate of an arrival       | story/greet.yaml           | TRIGGER_LEAK      | leak                                                     |
      | max_stacks 0                        | rules/modifiers.yaml       | SCHEMA            | max_stacks below 1                                       |
      | a modifier of no meter              | rules/modifiers.yaml       | SCHEMA            | meter 'warmth'                                           |
      | a modifier for 0 ticks              | rules/verbs.yaml           | SCHEMA            | a modifier effect has ticks below 1                      |
      | a modifier listed twice             | rules/modifiers.yaml       | SCHEMA            | modifiers.yaml:6: modifier 'fatigue' is listed twice     |
      | an undefined modifier               | rules/verbs.yaml           | SCHEMA            | modifier 'scorch', which is not in rules/modifiers.yaml  |
      | a story meter the world declares    | story/ending.yaml          | SCHEMA            | meter 'dawn' is already in world/meters.yaml             |
      | a story meter a cast declares       | story/ending.yaml          | SCHEMA            | already declared in another cast, place or story file    |
      | a story meter on the wrong story    | story/prologue.yaml        | SCHEMA            | but only story:ending carries meter 'dread'              |
      | an undeclared place meter           | rules/verbs.yaml           | SCHEMA            | place:docks_street carries no meter 'light'              |
      | a repeated story id                 | story/later.yaml           | SCHEMA            | story 'ending' is already defined                        |
      | an empty story id                   | story/ending.yaml          | SCHEMA            | id is empty                                              |
      | a walker on a missing exit          | state/save.yaml            | SCHEMA            | 'exit:docks_shop.ladder', which the pack does not define |
      | a verb named cancel                 | rules/verbs.yaml           | SCHEMA            | 'cancel' is a built-in command                           |
      | a saved item with no definition     | state/save.yaml            | SCHEMA            | unknown item 'ghost'                                     |
      | partial_at without partial          | rules/verbs.yaml           | SCHEMA            | without a partial                                        |
      | partial without partial_at          | rules/verbs.yaml           | SCHEMA            | without partial_at                                       |
      | partial_at above success_at         | rules/verbs.yaml           | SCHEMA            | at or above success_at                                   |
      | partial without a check             | rules/verbs.yaml           | SCHEMA            | outcome without a check                                  |
      | a nested partial_at without partial | rules/verbs.yaml           | SCHEMA            | rule 'press' sets partial_at without a partial list      |
      | a cast id world                     | cast/world.md              | SCHEMA            | rename this cast member                                  |
      | a saved actor world                 | state/save.yaml            | SCHEMA            | actor id world is reserved                               |

  Scenario: Two problems in two files are both reported
    Given a docks table
    When the copy is broken by "missing when"
    And the copy is broken by "exit to an unknown place"
    And the operator runs spook check
    Then the check exits 1
    And the report names "rules/verbs.yaml"
    And the report names "world/map.yaml"
    And stdout is empty
    And the pack bytes are unchanged

  Scenario: A clean pack prints an empty JSON array
    Given a docks table
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: A broken pack prints JSON diagnostics
    Given a docks table
    When the copy is broken by "missing meters"
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And a JSON diagnostic has code "NOT_IN_SPEC" file "rules/meters.yaml" path "schema" and null line and column
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: A component on an actor with no control type is refused
    Given a docks table
    When the save gives meters to an actor with no control type
    And the operator runs spook check --json
    Then the check exits 1
    And the JSON array is one diagnostic code "SCHEMA" file "state/save.yaml" path "actors.meters.nobody"

  Scenario: JSON diagnostics stay in the same order
    Given a docks table
    When the copy is broken by "missing when"
    And the copy is broken by "exit to an unknown place"
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And the JSON diagnostics are ordered by file, line, and path
    When the operator runs spook check --json again
    Then stdout matches the earlier JSON
    And stderr is empty
    And the pack bytes are unchanged

  Scenario: An unreachable place is a warning and the check still exits 0
    Given a docks table
    When the street exit is removed
    And the pack_rev is restamped
    And the operator runs spook check
    Then the check exits 0
    And the report names "WARNING"
    And the report says "no path"
    And stdout is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout is a JSON array of diagnostics
    And a JSON diagnostic has code "WARNING" file "world/places" path "docks_street" and null line and column
    And stderr is empty

  Scenario Outline: Teleport in verbs.yaml is a warning and the check still exits 0
    Given a docks table
    When travel in verbs.yaml teleports <walk>
    And the pack_rev is restamped
    And the operator runs spook check
    Then the check exits 0
    And the report names "WARNING"
    And the report says "teleports"
    And stdout is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout is a JSON array of diagnostics
    And a JSON diagnostic has code "WARNING" file "rules/verbs.yaml" path "rules[1]" and null line and column
    And stderr is empty

    Examples:
      | walk                  |
      | at once               |
      | inside a nested check |

  Scenario Outline: A flag <reader> names that no rule sets and no save holds is a warning
    Given a docks table
    When <reader> names flag "sealed", which no rule sets
    And the pack_rev is restamped
    And the operator runs spook check
    Then the check exits 0
    And the report names "WARNING"
    And the report says "flag 'sealed', which no rule sets and no save holds"
    When the operator runs spook check --json
    Then the JSON array is one diagnostic code "WARNING" file "<file>" path "<path>"
    When the save gives rowan the flag "sealed"
    And the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"

    Examples:
      | reader                | file             | path                 |
      | a verb's if           | rules/verbs.yaml | rules[8].if          |
      | an exit's revealed_by | world/map.yaml   | paths[7].revealed_by |

  Scenario: Long gm_notes are a warning and the check still exits 0
    Given a docks table
    When the pack's gm_notes run to 1001 characters
    And the operator runs spook check
    Then the check exits 0
    And the report names "WARNING"
    And the report names "gm_notes"
    And the report says "1000 characters"
    And the report says "tone"
    And stdout is empty
    When the operator runs spook check --json
    Then the check exits 0
    And the JSON array is one diagnostic code "WARNING" file "pack.yaml" path "gm_notes"
    And stderr is empty

  Scenario: The authoring example move passes check
    Given a docks table
    When the worked example row is appended to the verbs file
    And the operator runs spook check --json
    Then the check exits 1
    And stdout is a JSON array of diagnostics
    And the JSON array is one diagnostic code "PACK_REV_MISMATCH" file "state/save.yaml" path "pack_rev"
    When the operator runs migrate
    Then the command exits 0
    And stderr is empty
    When the operator runs spook check --json
    Then the check exits 0
    And stdout says "[]"
    And stderr is empty
