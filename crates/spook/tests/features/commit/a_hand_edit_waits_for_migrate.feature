Feature: A hand edit waits for migrate

  Scenario: A canon byte change writes nothing
    Given a docks table
    When a canon file gains a byte
    And the operator tries a round
    Then the command exits 1
    And the round writes nothing

  Scenario: Two migrates bump state_rev and append evidence
    Given a docks table
    When the operator migrates twice
    Then state_rev is 2
    And world_time is 1080
    And round index is 0
    And no round record was written
    And the journal has two migrate records, at state_rev 1 and 2

  Scenario: Migrate keeps a hand edit and rewrites pack_rev
    Given a docks table
    When the save is hand edited and the canon gains a byte
    And the pack adds meter "stamina" with start 6
    And the operator runs migrate
    Then the command exits 0
    And stdout is empty
    And stderr is empty
    And migrate keeps the hand edit
    When the operator runs spook check
    Then the check exits 0
    And spook verify passes
    And every actor has "stamina" at 6

  Scenario: A schema failure writes nothing
    Given a docks table
    When the save is replaced with a broken document
    And the operator runs migrate
    Then the command exits 1
    And stderr is SCHEMA
    And the pack bytes are unchanged

  Scenario: A canon edit after a committed round is refused
    Given a docks table
    And one round has committed
    When a canon file gains a byte
    And the operator tries a round
    Then the command exits 1
    And stderr names PACK_REV_MISMATCH
    And the round writes nothing

  Scenario: Check and run agree on an edit after a round
    Given a docks table
    And one round has committed
    When a canon file gains a byte
    Then spook check reports PACK_REV_MISMATCH
    And the host refuses to start with PACK_REV_MISMATCH

  Scenario: Migrate after rounds lets the next round run
    Given a docks table
    And one round has committed
    When a canon file gains a byte
    And the pack adds meter "stamina" with start 6
    And the operator runs migrate
    Then the command exits 0
    When the operator runs spook check
    Then the check exits 0
    And spook verify passes
    And every actor has "stamina" at 6
    When the operator tries a round
    Then the command exits 0
    And spook verify passes

  Scenario: A canon edit after a migrate is refused
    Given a docks table
    When a canon file gains a byte
    And the operator runs migrate
    And the operator tries a round
    Then the command exits 0
    When a canon file gains a byte
    And the operator tries a round
    Then the command exits 1
    And stderr names PACK_REV_MISMATCH
    And the round writes nothing

  Scenario: A second canon edit in one live bundle migrates
    Given a docks table
    And one round has committed
    When a canon file gains a byte
    And the operator runs migrate
    And the operator tries a round
    And a canon file gains a byte
    And the pack adds meter "stamina" with start 6
    And the operator runs migrate
    Then the command exits 0
    When the operator runs spook check
    Then the check exits 0
    And spook verify passes
    And every actor has "stamina" at 6
    When the operator tries a round
    Then the command exits 0
    And spook verify passes

  Scenario: A meter missing from the save with no migrate is refused
    Given a docks table
    When the save drops meter "rest"
    And the operator runs spook check
    Then the check exits 1
    And stderr names SCHEMA
    And the host refuses to start with SCHEMA

  Scenario: Verify replays a migrate from before a later canon edit
    Given a docks table
    And checkpoints are taken every 2 rounds
    When a canon file gains a byte
    And the operator runs migrate
    And the operator tries a round
    And the operator tries a round
    And a canon file gains a byte
    And the operator runs migrate
    And the operator tries a round
    Then the command exits 0
    And spook verify passes

  Scenario: Migrate keeps a hand edit at a checkpoint
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 2 rounds
    And the shop's lamp oil is hand edited to 2
    And the operator runs migrate
    Then the command exits 0
    And the shop has 2 lamp oil
    And verify trusts the operator edit at state_rev 3
    When the operator runs quick verify
    Then the command exits 0
    When the table is opened again
    Then the shop has 2 lamp oil
    When the operator tries a round
    Then the command exits 0
    And the shop has 2 lamp oil
    And spook verify passes

  Scenario: Migrate keeps a hand edit before any checkpoint
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the shop's lamp oil is hand edited to 2
    And the operator runs migrate
    Then the command exits 0
    And the shop has 2 lamp oil
    And spook verify passes
    When the operator tries a round
    Then the command exits 0
    And the shop has 2 lamp oil
    And spook verify passes

  Scenario: A hand edit behind the journal tip is refused
    Given a docks table
    And checkpoints are taken every 2 rounds
    When the operator plays 3 rounds
    And the shop's lamp oil is hand edited to 2
    And the operator runs migrate
    Then the command exits 1
    And stderr names HAND_EDIT
    And the pack bytes are unchanged
    When the operator runs spook check
    Then stderr names HAND_EDIT
    And the host refuses to start with HAND_EDIT

  Scenario: A plain migrate keeps its bytes
    Given a docks table
    When the operator runs migrate
    Then the command exits 0
    And the journal line migrate-0001 is
      """
      {"kind":"migrate","pack_rev_before":"blake2s:94d91cbe58bbffb352642733b49b025c821278a3ffbb8e3df30c1fd2ce8915bd","pack_rev_after":"blake2s:94d91cbe58bbffb352642733b49b025c821278a3ffbb8e3df30c1fd2ce8915bd","state_rev_before":0,"state_rev_after":1,"state_hash":"blake2s:c56a7ff71b490de2148b43e9f76bc41b01c1a6b57512fe09b433cd59117ea92c","prev":"blake2s:7e23e440f3f4c2ed1ed46cef768c2ef80ceccd0163dffaa605e31299740a505a","chain":"blake2s:29a3473b6d0512d87177825645c1ff3245c343f54679434b5d7191a95022e3c8"}
      """

  Scenario: Migrate gives a meter only to the actors that lack it
    Given a docks table
    When Rowan's grit offset is hand edited to 3 and Nell joins without grit
    And the operator runs migrate
    Then the command exits 0
    And pc_rowan's grit is at base 1 and offset 3
    And pc_nell's grit is at base 1 and offset 0
    And spook verify passes
