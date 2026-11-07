Feature: The AI GM connects as the GM

  Scenario: The AI GM with an extra argument is a usage error
    Given a docks table
    When the operator runs ai-gm with an extra argument
    Then the command exits 2
    And stderr shows usage
    And stdout is empty
    And nothing was dialed

  Scenario: The AI GM with nothing listening leaves the pack
    Given a docks table
    When the operator runs ai-gm with nothing listening
    Then the command exits 1
    And the pack bytes are unchanged

  Scenario Outline: A missing model setting does not dial
    Given a docks table
    When the operator runs ai-gm without "<name>"
    Then the command exits 1
    And stderr names "<name>" as unset
    And stdout is empty
    And the listener accepted no connection

    Examples:
      | name           |
      | SPOOK_GM_KEY   |
      | SPOOK_GM_URL   |
      | SPOOK_GM_MODEL |

  Scenario: A hung model call times out and the table keeps going
    When a hung model call meets a queued player line
    Then the queued line committed one round
    And the hung call retried once after the delay
    And both hung calls closed
    And the seat stayed up

  Scenario: A model that answers 500 once is retried and the say lands
    When a model answers 500 once and then a say
    Then the stub saw 3 model posts
    And the model's say landed
    And the queued line committed one round

  Scenario: A model that answers 401 is not retried
    When a model answers 401
    Then the stub saw 1 model post
    And the model's say did not land
    And the queued line committed one round
    And the error line shows the provider's text without the key

  Scenario: The AI GM seat submits a say and reports a refused call
    Given a docks table with a seated player
    When the GM seat says good evening and calls an unknown tool
    Then the player saw a say from the GM
    And the tape from is gm
    And the GM call was refused

  Scenario: A scripted AI GM follows the turn rules
    When the scripted AI GM plays the turn checks

  Scenario: Play, the AI GM, and a stub share one speech turn and one travel turn
    When play and the AI GM and a stub run a speech turn and a travel turn

  Scenario: A move queued while the model answers a say gets its own turn
    When a player queues travel while the model answers their say
    Then the stub saw 2 model posts
    And the queued line committed one round

  Scenario: The AI GM dials the game port
    Given a docks table
    When the AI GM is pointed at a game port
    Then the AI GM dialed that address
