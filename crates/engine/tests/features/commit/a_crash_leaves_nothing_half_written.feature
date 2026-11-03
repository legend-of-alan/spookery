Feature: A crash leaves nothing half written

  Scenario: A queued move that never commits writes nothing
    Given a docks table
    When a player queues take lamp_oil and the host stops before a round
    Then the save bytes are unchanged
    And the next host opens at the same state_rev with the same lamp oil

  Scenario: Stray temp files are removed when the pack is opened
    Given a docks table
    When a save.yaml.tmp and a bundle-0000.jsonl.gz.tmp are left under state/
    And the table is opened again
    Then no .tmp file remains under state/
    And the save bytes are unchanged
