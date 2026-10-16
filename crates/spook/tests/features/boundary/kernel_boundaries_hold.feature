Feature: Kernel boundaries hold

  Scenario: Spook only uses the kernel API
    Given a spook source file that imports spookery_kernel::shard
    Then the boundary check fails naming that file and path

  Scenario: Subsystems import downward only
    Given kernel/state imports crate::rules
    Then the boundary check fails

  Scenario: Clean tree passes
    Then the boundary check passes on main
