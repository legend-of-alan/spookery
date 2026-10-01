@v0
Feature: Proposal only
  Proof: v0_t10_proposal_only
  Idle delete-place writes state/proposals/<id>.yaml and leaves the place markdown in place.

  Scenario: Idle delete-place writes a proposal and leaves the place markdown in place
    Given a temp copy of pack packs/oneshot-docks
    When Mira files an idle delete_place proposal for docks_shop
    Then proposal delete_docks_shop.yaml exists
    And the docks_shop place markdown is unchanged
