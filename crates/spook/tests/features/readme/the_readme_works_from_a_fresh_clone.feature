Feature: The README works from a fresh clone

  The quick-start job runs these commands on a clean runner for each OS.
  This process runs them when it is that OS.

  Scenario Outline: Quick start reaches an ending
    Given a clean <os> runner and a fresh clone
    When the README quick-start commands run with the stub GM
    Then the session reaches the ending and spook verify passes

    Examples:
      | os      |
      | ubuntu  |
      | macos   |
      | windows |

  Scenario: Docs say how this differs
    Then the docs have one how-this-differs paragraph
