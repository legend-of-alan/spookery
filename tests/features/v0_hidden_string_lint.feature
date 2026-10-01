@v0
Feature: Hidden-string lint
  Proof: v0_c12_hidden_string_lint
  Hidden-string lint on bodies.

  Scenario: Hidden-string lint finds docks bodies hide no ledger sentence
    Given a temp copy of pack packs/oneshot-docks
    When the pack is linted
    Then the lint hits are empty

  Scenario: Hidden-string lint does not exempt the owner body
    Given a temp copy of pack packs/oneshot-docks
    When "The lamp-oil ledger is short." is appended to "cast/mira.md"
    And the pack is linted
    Then a lint hit is path "cast/mira.md" and text "The lamp-oil ledger is short."

  Scenario: Hidden-string lint does not skip the place body
    Given a temp copy of pack packs/oneshot-docks
    When "The lamp-oil ledger is short." is appended to "world/places/docks_shop.md"
    And the pack is linted
    Then a lint hit is path "world/places/docks_shop.md" and text "The lamp-oil ledger is short."

  Scenario: Hidden-string lint ignores a case-folded ledger sentence
    Given a temp copy of pack packs/oneshot-docks
    When "the lamp-oil ledger is short." is appended to "cast/mira.md"
    And the pack is linted
    Then the lint hits are empty
