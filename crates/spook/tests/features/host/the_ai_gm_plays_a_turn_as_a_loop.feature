Feature: The AI GM plays a turn as a loop

  Scenario: The GM perceives before it acts, and reads prose but no fact body
    Given a docks table with a seated player and the NPC Mira in the shop
    And a scripted model that perceives as Mira, then has Mira say hello, then ends the turn
    When the player says "evening" in the shop
    Then the table saw a perceive as Mira before any say
    And the model read Mira's prose and her hidden fact's text, but no fact body
    And the player saw Mira say hello

  Scenario: The GM recovers after a refusal
    Given a docks table with a seated player and the NPC Mira in the shop
    And a scripted model that queues a move Mira can't make
    When the table refuses the queue
    Then the model's next call carries the refusal as the tool result
    And the scripted model queues wait for Mira and ends the turn
    And one round is committed

  Scenario: The GM runs several NPCs in one round
    Given a docks table with a seated player and the NPCs Mira and Aldric in the shop
    And a scripted model that queues a move for Mira and a move for Aldric, then ends the turn
    When the player queues travel
    Then the round resolves the player's, Mira's and Aldric's moves together
    And the player sees the GM narrate the result, not the dice

  Scenario: The model call limit commits for a stuck model
    Given a docks table with a seated player who has queued a move
    And a scripted model that only ever calls perceive
    When the GM's turn runs
    Then the client stops after 20 model calls
    And the client commits the round itself
    And the GM reports a model call limit warning

  Scenario: A quiet table outlasts the graph's task timeout
    Given a docks table with a seated player and the AI GM connected
    When the table stays quiet for longer than the graph's task timeout
    And the player says "evening" in the shop
    Then the scripted model is called for that say
    And the AI GM is still connected
