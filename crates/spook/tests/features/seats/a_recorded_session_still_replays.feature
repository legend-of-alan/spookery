Feature: A session from the previous release still replays

  A client ignores kinds and fields it does not know, and the wire only adds, so every
  message a recorded session got still reaches it.

  Scenario: A recorded session replays against this build
    Given a session recorded against the previous release
    When CI replays it against this build
    Then every server message the old client knows is unchanged
