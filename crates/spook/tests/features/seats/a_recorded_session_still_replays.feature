Feature: A recorded session still replays

  A client ignores kinds and fields it does not know, and the wire only adds, so every
  message a recorded session got still reaches it.

  Scenario: A recorded session replays against this build
    Given a session recorded against this wire version
    When CI replays it against this build
    Then every server message the old client knows is unchanged
