@v01
Feature: Attempt on round close
  Proof: v01_attempt_line_refuses_before_a_roll, v01_p09_no_gm_attempt_rolls_on_send_and_writes_no_now_path, v01_p09_gm_attempt_rolls_on_commit_and_the_gm_sees_faces, v01_unknown_check_does_not_advance_the_die_or_send_a_check, v01_player_lines_do_not_commit
  The roll writes no Now path. The tick JSONL gains no faces, stat, total, target, or success field. args may contain check and target. Pause policy and NPC derivation still run as v0. The player event's success is a boolean. The GM event has faces. attempt sneak shadow is refused.

  Scenario: Attempt law refuses a line before a roll
    When an unseated attempt shard receives attempt grit 3
    Then attempt presence humans is 0
    And the attempt refusal is TABLE_EMPTY
    And the attempt queue is empty
    And the attempt pack has no session.jsonl
    And the attempt pack has no tick-0001.jsonl
    And the attempt now.yaml matches the before bytes
    When the attempt shard seats pc_rowan
    Then attempt and attempt grit are SCHEMA and write nothing
    When the seated attempt shard receives attempt sneak shadow
    Then the attempt refusal is NOT_IN_SPEC
    And the attempt queue is empty
    And the attempt pack has no tick-0001.jsonl
    And the attempt pack has no session.jsonl
    And the attempt now.yaml matches the seated bytes
    And expand of move attempt is empty
    And expand of moves attack, give, and take is NOT_IN_SPEC
    And the attempt session ends

  Scenario: Attempt law rolls on send when no GM is seated
    Given the attempt port is locked and the shard is listening
    When the attempt player connects and perceives
    And the attempt player sends attempt grit 3 with no GM
    Then the player checks are one success true
    And the player stdout has no faces
    And the attempt player stderr does not contain NOT_IN_SPEC
    And attempt presence humans is 1
    And attempt world_time is 1085
    And attempt tick_index is 1
    And pc_rowan.yaml matches the attempt sheet
    And tick-0001.jsonl has the attempt commit keys in order and no roll fields
    And the attempt intent args are check grit and target 3
    And the attempt tool_trace contains resolve_check
    When a twin shard seats pc_rowan queues wait and commits
    Then assassination_debt is not paused after the attempt
    And the twin now matches the attempt now on the v0 paths
    And the twin kernel moves match the attempt tick
    When the attempt player sends attempt grit 14
    Then the player checks are success true then success false
    And pc_rowan.yaml matches the attempt sheet
    And the attempt session ends

  Scenario: Attempt law rolls on GM commit and the GM sees faces
    Given the attempt port is locked and the shard is listening
    When an attempt GM session is open
    And the attempt player connects and perceives
    And the attempt player sends attempt grit 3 while a GM is seated
    Then the player stdout after perceive is queued without check or resume
    And attempt world_time is 1080
    And the attempt pack has no tick-0001.jsonl
    When the GM commits the attempt turn
    Then the GM sees one check event after the commit result
    And the commit result has no faces
    And the GM check event lists faces before stat total target and success
    And the GM roll has two faces stat 1 and target 3
    And the player check success matches the GM roll
    And attempt world_time is 1085
    And the attempt session ends

  Scenario: Attempt law unknown check does not advance the die
    Given the attempt port is locked and the shard is listening
    When an attempt GM session is open
    And the attempt player connects and perceives
    And the attempt player sends attempt nope 5
    And the attempt player sends a second queued attempt grit 3
    Then the player stdout has two queued lines and no check
    When the GM commits the attempt turn after the unknown check
    Then the GM sees one check event
    And the attempt player stderr is UNKNOWN_KEY
    And the GM check event lists faces before stat total target and success
    And the GM roll has two faces stat 1 and target 3
    And the player check success matches the GM roll
    And tick-0001.jsonl has no roll fields outside args
    And tick-0001.jsonl rejects UNKNOWN_KEY and keeps only the grit attempt
    And tick-0001.jsonl resolve_check count is 2
    And the GM faces match a fresh grit resolve_check
    And the attempt session ends

  Scenario: Attempt law refuses attempt sneak shadow without a commit
    Given the attempt port is locked and the shard is listening
    When the attempt player connects and perceives
    And the seated attempt now.yaml is recorded
    And the attempt player sends attempt sneak shadow
    Then the attempt player stderr contains NOT_IN_SPEC
    And the attempt now.yaml matches the seated bytes
    And the attempt player is still running
    And the attempt pack has no tick file
    And the attempt pack has no session.jsonl
    And the attempt session ends
