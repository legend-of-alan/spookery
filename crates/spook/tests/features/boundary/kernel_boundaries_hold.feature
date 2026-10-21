Feature: Kernel boundaries hold

  Scenario: Subsystems import downward only
    Given kernel/state imports crate::rules
    Then the boundary check fails

  Scenario: A former climb fails
    Given kernel/canon.rs imports crate::map
    Then the boundary check names crate::map

  Scenario: Clean tree passes
    Then the boundary check passes on main
