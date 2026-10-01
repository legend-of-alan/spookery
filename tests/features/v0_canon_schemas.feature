@v0
Feature: Canon schemas
  Proof: v0_canon_schemas_reject_unknown_keys
  Schemas reject unknown keys.

  Scenario: Canon schemas reject unknown keys
    Then schema place rejects unknown keys
    And schema cast rejects unknown keys
    And schema moves rejects unknown keys
    And schema checks rejects unknown keys
    And schema tick rejects unknown keys
    And schema sheet rejects unknown keys
    And schema proposal rejects unknown keys
