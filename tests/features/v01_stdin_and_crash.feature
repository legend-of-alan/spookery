@v01
Feature: Stdin and crash presence
  Proof: v01_p14_stdin_is_not_a_play_door, v01_p14_stop_and_eof_drop_a_held_seat_and_do_not_tick, v01_p16_crashed_presence_clears_before_accept_and_other_commands_do_not
  Stdin wait on spook run prints NOT_IN_SPEC, does not exit, and writes nothing. With a player connection held, stop and stdin EOF drop that seat, commit include_tick false, do not call run_tick, and exit 0. With a GM connection held, stop frees it and writes no presence. With neither connection held, stop exits 0 and writes nothing. A crashed humans 1 and session id is cleared by spook run before accept: humans is 0, session_ids is empty, and there is no tick file. The next player hello sets humans to 1. The following wake is idle catch-up. spook tick on a copy leaves humans 1 and ticks live. status, migrate, and run --mcp leave the crashed presence.

  Scenario: Stdin law rejects wait and stop with no seat writes nothing
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And the pack file bytes are recorded
    When spook run is listening
    And stdin sends wait
    Then the run stderr is NOT_IN_SPEC
    And the run stdout is empty
    And the run process is still running
    And the recorded pack file bytes are unchanged
    And state/log holds 0 tick jsonl files
    When stdin sends stop
    Then the run exits 0
    And the recorded pack file bytes are unchanged

  Scenario: Stdin law stop drops a held player seat and does not tick
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    When spook run is listening
    And spook play is seated
    Then state_rev is 1
    When stdin sends stop
    Then the run exits 0
    And humans is 0
    And session_ids are empty
    And state_rev is 2
    And tick_index is 0
    And world_time is 1080
    And state/log holds 0 tick jsonl files

  Scenario: Stdin law EOF drops a held player seat and does not tick
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    When spook run is listening
    And spook play is seated
    When the run stdin reaches EOF
    Then the run exits 0
    And humans is 0
    And state_rev is 2
    And state/log holds 0 tick jsonl files

  Scenario: Stdin law stop frees a held GM seat and writes no presence
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    When a GM seat is held on spook run
    Then the second GM hello is SESSION_FULL
    And now.yaml bytes equal the pre-GM snapshot
    When stdin sends stop
    Then the run exits 0
    And now.yaml bytes equal the pre-GM snapshot

  Scenario: Stdin law tick status migrate and mcp leave crashed presence
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And crashed presence copies for tick, status, mcp, and migrate
    Then the tick copy opens with crashed presence
    When the tick copy is ticked
    Then the tick copy command succeeds
    And the tick copy humans is 1
    And the tick copy tick_index is 1
    And the tick copy world_time is 1085
    And the tick copy tick log is tick-0001.jsonl
    When status runs on the status copy
    Then the status copy command succeeds
    And the status copy humans is 1
    And the status copy state_rev is 0
    When mcp pack-table runs on the mcp copy
    Then the mcp copy is still running
    And the mcp copy humans is 1
    And the mcp copy session_ids are crash-session
    When migrate runs on the migrate copy
    Then the migrate copy command succeeds
    And the migrate copy humans is 1
    And the migrate copy session_ids are crash-session

  Scenario: Stdin law run clears crashed presence and the wake is idle catch-up
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And the pack now.yaml is a crashed presence
    When spook run is listening
    Then humans is 0
    And session_ids are empty
    And actors_present are empty
    And state_rev is 1
    And tick_index is 0
    And world_time is 1080
    And state/log holds 0 tick jsonl files
    And last_tick_at is 2026-01-01T00:00:00Z
    When spook play is seated
    Then humans is 1
    When the player stdin reaches EOF
    Then the player exits 0
    And humans returns to 0
    And tick_index is 0
    And state/log holds 0 tick jsonl files
    When stdin sends stop
    Then the run exits 0
    When the scheduler wakes the pack
    Then humans is 0
    And tick_index is 12
    And world_time is 1800
