@v01
Feature: Player seat drop
  Proof: v01_p05_disconnect_and_close_drop_the_player_seat_without_a_tick, v01_p17_quiet_socket_stays_seated, v01_p17_simulated_read_error_drops_the_seat_like_close
  Player disconnect returns humans to 0, one state_rev bump, and no tick file.
  A simulated read error drops the seat the way close does.
  A held socket that has sent no player line, and has not errored, is still seated.
  The quiet-socket wait does not sleep for tick_every_real_seconds.

  Scenario: Seat drop on disconnect and close leaves no tick file
    Given a temp copy of pack packs/oneshot-docks
    And the player seat port is locked and the shard is listening
    When the player client connects and perceives
    Then presence humans is 1
    When the player stdin closes
    Then the player client exits 0
    And humans returns to 0
    And state_rev is 2
    And tick_index is 0
    And world_time is 1080
    And state/log holds 0 tick jsonl files
    When the player client connects and perceives
    And the player sends close
    Then the player client exits 0
    And humans returns to 0
    And state_rev is 4
    And state/log holds 0 tick jsonl files
    And state/log/session.jsonl does not exist

  Scenario: Seat drop keeps a quiet socket seated
    Given a temp copy of pack packs/oneshot-docks
    And the player seat port is locked and the shard is listening
    When the player client connects and perceives
    And one second passes with no player line
    Then that wait stayed under 30 seconds
    And presence humans is 1
    And state_rev is 1
    And state/log holds 0 tick jsonl files
    And the player client is still running

  Scenario: Seat drop on a read error matches close
    Given a temp copy of pack packs/oneshot-docks
    When the player seat opens for pc_rowan
    Then presence humans is 1
    And state_rev is 1
    And state/log holds 0 tick jsonl files
    When the player seat ends with a read error
    Then presence humans is 0
    And actors_present is empty
    And state_rev is 2
    And tick_index is 0
    And world_time is 1080
    And state/log holds 0 tick jsonl files
    When the player seat opens for pc_rowan
    And the player seat ends with close
    Then presence humans is 0
    And state_rev is 4
    And state/log holds 0 tick jsonl files
    When the player seat opens for pc_rowan
    And the player seat ends with keepalive
    Then presence humans is 0
    And state_rev is 6
    And tick_index is 0
    And state/log holds 0 tick jsonl files
    And state/log/session.jsonl does not exist
