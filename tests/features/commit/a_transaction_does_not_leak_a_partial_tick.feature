Feature: A transaction does not leak a partial tick

  Scenario: Dropping an open transaction leaves the save unchanged
    When an open transaction is dropped

  Scenario: An incomplete transaction is discarded
    When an incomplete transaction is opened

  Scenario: A finished transaction at the same revision restores the previous save
    When a finished transaction repeats the current revision

  Scenario: A finished transaction at the next revision keeps the new save and drops orphans
    When a finished transaction advances the revision
