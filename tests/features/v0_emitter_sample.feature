@v0
Feature: Emitter sample
  Proof: v0_emitter_sample_is_166_bytes
  The emitter sample is 166 bytes.

  Scenario: The emitter sample is 166 bytes
    When the now emitter sample is rendered
    Then the emitter sample is 166 bytes
    And the emitter sample equals the canon text
