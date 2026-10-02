@v02
Feature: Rowan travels to the street
  Proof: v02_p09_rowan_travels_to_the_street
  With a GM seated, speech and wait stay in docks_shop until commit, travel mira_rooms refuses OUT_OF_RANGE, and travel docks_street moves Rowan. With no GM, those lines commit one live tick and world_time advances by 5.

  Scenario: Street GM Rowan travels to the street
    Given the street port is locked
    When a street player sits with a GM on shard "street-gm"
    Then street humans is 1
    And street world_time is 1080
    And street tick_index is 0
    And street Rowan is in docks_shop
    And street has no tick log
    And the street perceive place is "docks_shop"
    When the street player enters "say the tin is out"
    Then the street player stdout contains "the tin is out"
    When the street player enters "talk mira good evening"
    Then the street player stdout contains "good evening"
    And street world_time is 1080
    And street tick_index is 0
    And street Rowan is in docks_shop
    And street has no tick log
    When the street player enters "wait"
    Then the street player queued count is 1 with no resume
    And street world_time is 1080
    And street Rowan is in docks_shop
    And street has no tick log
    And the street session queue is only the last queued id as "wait"
    When the street GM commits the turn
    Then the street commit has no code
    And street world_time is 1085
    And street tick_index is 1
    And street Rowan is in docks_shop
    And street tick "tick-0001.jsonl" records the last queued id as player move "wait"
    And street tick "tick-0001.jsonl" has no derived pc_rowan kernel intent
    When the street player enters "travel mira_rooms"
    Then the street player queued count is 2 with no resume
    And street world_time is 1085
    And street Rowan is in docks_shop
    And street tick logs are "tick-0001.jsonl"
    When the street GM commits the turn
    Then the street commit has no code
    And street world_time is 1090
    And street tick_index is 2
    And street Rowan is in docks_shop
    And street tick "tick-0002.jsonl" rejects the last queued id as "OUT_OF_RANGE"
    And street tick "tick-0002.jsonl" has no pc_rowan location effect
    And street tick "tick-0002.jsonl" has no derived pc_rowan kernel intent
    When the street player enters "travel docks_street"
    Then the street player queued count is 3 with no resume and no street place
    And street world_time is 1090
    And street tick_index is 2
    And street Rowan is in docks_shop
    And the street session queue is only the last queued id as "travel"
    When the street player stdout is marked
    And the street world_time is marked
    And the street GM commits the turn
    Then the street commit has no code
    And the street player stdout gains the street place
    And the street addition contains kind perceive
    And street world_time is 1095
    And street world_time advanced by 5 from the mark
    And street tick_index is 3
    And street humans is 1
    And the street scene is docks_street
    And street tick "tick-0003.jsonl" ran from 1090 to 1095
    And street tick "tick-0003.jsonl" moved Rowan from docks_shop to docks_street on the last queued id
    And street tick "tick-0003.jsonl" has no derived pc_rowan kernel intent
    And the street session stops

  Scenario: Street live Rowan travels to the street
    Given the street port is locked
    When a street player sits on shard "street-live"
    Then street world_time is 1080
    And street Rowan is in docks_shop
    And the street perceive place is "docks_shop"
    When the street player enters "say the tin is out"
    Then the street player stdout contains "the tin is out"
    When the street player enters "talk mira good evening"
    Then the street player stdout contains "good evening"
    And street world_time is 1080
    And street tick_index is 0
    And street Rowan is in docks_shop
    And street has no tick log
    When the street player enters "wait"
    Then the street player gained a resume and no queued line
    And street world_time is 1085
    And street tick_index is 1
    And street Rowan is in docks_shop
    When the street player enters "travel mira_rooms"
    Then the street player gained a resume and no queued line
    And street world_time is 1090
    And street tick_index is 2
    And street Rowan is in docks_shop
    And street tick "tick-0002.jsonl" ran from 1085 to 1090
    And street tick "tick-0002.jsonl" has one player rejection "OUT_OF_RANGE"
    And street tick "tick-0002.jsonl" has no pc_rowan location effect
    And street tick "tick-0002.jsonl" has no pc_rowan travel intent
    And street tick "tick-0002.jsonl" has no derived pc_rowan kernel intent
    When the street world_time is marked
    And the street player enters "travel docks_street"
    Then the street player sees the street with resume and perceive and no queued line
    And street humans is 1
    And street world_time is 1095
    And street world_time advanced by 5 from the mark
    And street tick_index is 3
    And the street scene is docks_street
    And street tick "tick-0003.jsonl" ran from 1090 to 1095
    And street tick "tick-0003.jsonl" has a player travel to docks_street with a location effect from docks_shop
    And street tick "tick-0003.jsonl" has no derived pc_rowan kernel intent
    And the street session stops
