Feature: A transaction does not leak a partial tick

  Scenario: Dropping an open transaction leaves the save unchanged
    When an open transaction is dropped

  Scenario: An incomplete transaction is discarded
    When an incomplete transaction is opened

  Scenario: A leftover transaction directory is removed and the save stays
    When a finished transaction repeats the current revision

  Scenario: A torn journal tail is cut off and an orphan memory line is dropped
    When a finished transaction advances the revision
