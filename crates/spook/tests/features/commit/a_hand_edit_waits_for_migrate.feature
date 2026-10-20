Feature: A hand edit waits for migrate

  Scenario: A canon byte change writes nothing
    Given a docks table
    When a canon file gains a byte
    And the operator runs tick
    Then the command exits 1
    And the tick writes nothing

  Scenario: Two migrates bump state_rev and append evidence
    Given a docks table
    When the operator migrates twice
    Then state_rev is 2
    And world_time is 1080
    And tick index is 0
    And no tick file was written
    And the studio log records two migrates

  Scenario: Migrate keeps a hand edit and rewrites pack_rev
    Given a docks table
    When the save is hand edited and the canon gains a byte
    And the operator runs migrate
    Then the command exits 0
    And stdout is empty
    And stderr is empty
    And migrate keeps the hand edit

  Scenario: A schema failure writes nothing
    Given a docks table
    When the save is replaced with a broken document
    And the operator runs migrate
    Then the command exits 1
    And stderr is SCHEMA
    And the pack bytes are unchanged
    And the studio log is absent

  Scenario: A canon edit after a committed round is refused
    Given a docks table
    And one round has committed
    When a canon file gains a byte
    And the operator runs tick
    Then the command exits 1
    And stderr names PACK_REV_MISMATCH
    And the tick writes nothing

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
    And the operator runs migrate
    And the operator runs tick
    Then the command exits 0
    And spook verify passes
