Feature: Spook modules have distinct names

  Scenario: No two modules share a file name
    Then no two spook modules share a file name besides mod.rs, lib.rs, and main.rs
