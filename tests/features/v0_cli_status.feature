@v0
Feature: CLI status
  Proof: v0_cli_status_prints_host_clocks_and_writes_nothing
  Status prints host clocks and writes nothing.

  Scenario: Status prints host clocks and writes nothing
    Given a temp copy of pack packs/oneshot-docks
    And the cli pack files are recorded
    When spook status runs on the pack
    Then the status command exits 0
    And the status command stdout is the host clocks
    And status leaves the cli pack files unchanged
