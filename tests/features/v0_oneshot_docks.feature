@v0
Feature: Oneshot docks
  Proof: v0_oneshot_docks_matches_section12
  The committed docks pack matches the recorded pack_rev, now.yaml, and pc_rowan.yaml bytes.

  Scenario: Oneshot docks pack bytes match the recorded pack_rev, now.yaml, and pc_rowan.yaml
    Given the source pack packs/oneshot-docks
    Then pack_rev is "blake2s:4fb2923546946116df5dec49198a50765588ac57b593f22b0cd266823c08710a"
    And "state/now.yaml" is 2006 bytes
    And "state/now.yaml" blake2s is "ea8497770f4b2ae6f5f330a932149bdfcb2732f6766c4e67411b73f35e482d46"
    And "state/now.yaml" round-trips through emit_now
    And now pack_rev is "blake2s:4fb2923546946116df5dec49198a50765588ac57b593f22b0cd266823c08710a"
    And "state/actors/pc_rowan.yaml" is 118 bytes
    And "state/actors/pc_rowan.yaml" blake2s is "0bc253962ff105876e0687c3649959cc58735e43b02f6ec43ca13167894f0e7b"
    And "state/actors/pc_rowan.yaml" round-trips through sheet emit
    And the shard opens
