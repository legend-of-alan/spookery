Feature: A hand edit waits for migrate

  Scenario: A canon byte change writes nothing
    Given a docks table
    When a canon file gains a byte
    And the operator tries a round
    Then the command exits 1
    And the round writes nothing

  Scenario: Two migrates bump state_rev
    Given a docks table
    When the operator migrates twice
    Then state_rev is 2
    And world_time is 1080
    And round index is 0
    And no round was played

  Scenario: Migrate keeps a hand edit and rewrites pack_rev
    Given a docks table
    When the save is hand edited and the canon gains a byte
    And the pack adds meter "stamina" with base 6
    And the operator runs migrate
    Then the command exits 0
    And stdout is empty
    And stderr is empty
    And migrate keeps the hand edit
    When the operator runs spook check
    Then the check exits 0
    And spook verify passes
    And every actor reads "stamina" at 6 with no row in the save

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
    And the pack adds meter "stamina" with base 6
    And the operator runs migrate
    Then the command exits 0
    When the operator runs spook check
    Then the check exits 0
    And spook verify passes
    And every actor reads "stamina" at 6 with no row in the save
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
    And the pack adds meter "stamina" with base 6
    And the operator runs migrate
    Then the command exits 0
    When the operator runs spook check
    Then the check exits 0
    And spook verify passes
    And every actor reads "stamina" at 6 with no row in the save
    When the operator tries a round
    Then the command exits 0
    And spook verify passes

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

  Scenario: An actor with no meter or act rows reads the pack's base
    Given a docks table
    When Rowan's vigor offset is hand edited to 3 and Nell joins with no meter or act rows
    And the operator runs migrate
    Then the command exits 0
    And rowan's vigor is at base 1 and offset 3
    And nell's vigor reads 1 and the save lists no vigor for her
    And the save lists no act row for nell
    And spook verify passes
