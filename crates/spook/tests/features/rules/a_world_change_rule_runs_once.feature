Feature: A world change rule runs once per change

  Scenario: Two seated players see one narrate and one add for one dawn crossing
    Given world/meters.yaml declares a "dawn" meter at 1
    And story/vigil.yaml has a change rule that watches dawn cross "<=0"
    And that rule narrates "The sun is up." and adds 1 to who "world" meter morning
    And pc_rowan and pc_nell are seated
    When a rule adds -1 to who "world" meter dawn
    Then the vigil rule fires once
    And pc_rowan and pc_nell each see "The sun is up." once
    And the world's morning is 1

  Scenario: With nobody seated the crossing still runs its rule
    Given world/meters.yaml declares a "dawn" meter at 1
    And story/vigil.yaml has a change rule that watches dawn cross "<=0" and adds 1 to who "world" meter morning
    And nobody is seated
    When a habit adds -1 to who "world" meter dawn
    Then the vigil rule fires once
    And the world's morning is 1
    And spook verify passes on this pack

  Scenario: The pack check refuses self in a world change rule
    Given a change rule watches world flag tide_turned and sets who "self" flag saw_tide
    When the author runs spook check
    Then it refuses the rule, saying a world change rule has no self

  Scenario: The pack check refuses a narrate with no to outside a world change rule
    Given a verb narrates "The bell rings." with no to
    When the author runs spook check
    Then it refuses the verb, saying its narrate has no to
