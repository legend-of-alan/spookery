@v0
Feature: Import boundaries
  Proof: v0_import_boundaries
  Import scan fails if src/kernel or src/tick contains rmcp, tokio, sqlx, postgres, or redis, or if any file under src contains langchain, openai, anthropic, ollama, or reqwest.

  Scenario: Import scan finds no forbidden client in kernel, tick, or src
    Then src/kernel and src/tick contain none of rmcp, tokio, sqlx, postgres, or redis
    And no file under src contains langchain, openai, anthropic, ollama, or reqwest
