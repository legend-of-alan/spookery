@v01
Feature: Loopback bind
  Proof: v01_run_as_is_not_in_spec, v01_p13_bind_other_than_loopback_is_not_in_spec, v01_p13_bind_in_use_is_the_operating_system_error, v01_p19_clients_without_a_listener_use_the_operating_system_error
  --bind 127.0.0.1 and --bind 127.0.0.1:<port> are legal. The default listen address is 127.0.0.1:9737. 0.0.0.0 and localhost exit 1 with NOT_IN_SPEC and do not open the pack. A failed bind of a port that is already held exits 1 with the operating-system error text on stderr, opens no pack, and writes nothing. It does not print NOT_IN_SPEC. spook play and spook gm --stdio with nothing listening exit 1 with the operating-system error, empty stdout, and an unchanged pack. spook gm without --stdio exits NOT_IN_SPEC and does not connect.

  Scenario: Bind law rejects spook run --as player:pc_rowan
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And the pack file bytes are recorded
    When spook run starts with --as player:pc_rowan
    Then the command exits 1
    And the command stderr is NOT_IN_SPEC
    And the command stdout is empty
    And the recorded pack file bytes are unchanged
    And port 9737 is free

  Scenario: Bind law rejects a bind address other than 127.0.0.1
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And the pack file bytes are recorded
    When spook run --bind is tried for 0.0.0.0 and localhost
    Then each command exits 1
    And each command stderr is NOT_IN_SPEC
    And each command stdout is empty
    And the recorded pack file bytes are unchanged
    When spook run starts with --mcp pack-table and --bind 127.0.0.1
    Then the command exits 1
    And the command stderr is NOT_IN_SPEC
    And the recorded pack file bytes are unchanged
    When spook run --bind 127.0.0.1:9737 listens
    Then that loopback bind was accepted
    And port 9737 is free

  Scenario: Bind law reports the operating-system error when 127.0.0.1:9737 is in use
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And the pack file bytes are recorded
    When spook run binds while 127.0.0.1:9737 is held
    Then the command exits 1
    And the command stdout is empty
    And the command stderr is the operating-system error
    And the command stderr does not contain NOT_IN_SPEC
    And the recorded pack file bytes are unchanged

  Scenario: Bind law reports the operating-system error when nothing is listening
    Given a temp copy of pack packs/oneshot-docks
    And the 9737 port lock is held
    And the pack file bytes are recorded
    And a directory that holds only that pack.yaml
    When spook play, spook play on the yaml-only directory, spook gm --stdio on the yaml-only directory, and spook play --as player:pc_rowan run with nothing listening
    Then each command exits 1
    And each command stdout is empty
    And each command stderr is the operating-system error
    And each command stderr does not contain NOT_IN_SPEC
    When spook gm runs with nothing listening
    Then the command exits 1
    And the command stdout is empty
    And the command stderr is NOT_IN_SPEC
    And the recorded pack file bytes are unchanged
