@v01
Feature: Speech
  Proof: v01_p06_say_while_gm_is_seated_appends_one_tape_line_without_a_tick, v01_p07_actor_say_is_on_the_gm_and_speaker_cursors_and_absent_for_a_later_actor, v01_p08_talk_appends_for_a_seated_player_and_live_only_when_no_human, v01_torn_tape_tail_is_not_a_record
  say while a GM is seated appends one tape line and does not tick. An actor-addressed say is on the GM and speaker cursors and absent for a later actor. talk appends for a seated player and is LIVE_ONLY when no human is seated. A torn tape tail is not a record.

  Scenario: Speech say while a GM is seated appends one tape line without a tick
    Given a temp copy of pack packs/oneshot-docks
    When the speech shard is listening
    And a gm client is seated
    And the speech player connects and perceives
    And the player says hello
    Then the player stdout contains the table say hello
    And the player stderr does not contain NOT_IN_SPEC
    And session.jsonl is the one table hello tape line
    And now.yaml matches the seated snapshot
    And state_rev world_time and tick_index match the seated snapshot
    And the pack has no tick file
    And the shard stdout contains the table hello tape line
    And the gm seat is still held
    And the speech processes stop

  Scenario: Speech actor say is on the GM and speaker cursors and absent for a later actor
    Given a temp copy of pack packs/oneshot-docks
    When the speech shard is listening
    And a gm client is seated
    And the speech player connects and perceives
    And the player talks to pc_rowan psst
    Then the player stdout contains the actor psst line
    And the gm stdout contains the actor psst line
    And the player stderr does not contain NOT_IN_SPEC
    And now.yaml matches the seated snapshot
    And world_time matches the seated snapshot
    And the pack has no tick file
    When the speech player stdin closes
    Then the speech player exits 0
    And humans returns to 0
    And state_rev is one more than the seated snapshot
    And world_time matches the seated snapshot
    And the pack has no tick file
    And pc_rowan scrollback contains the actor psst line
    And pc_later scrollback is empty and has no psst
    And the speaker pc_rowan is not the later actor pc_later
    And the speech processes stop

  Scenario: Speech talk appends for a seated player and is live only when no human
    Given a temp copy of pack packs/oneshot-docks
    Then presence humans is 0
    And session_ids is empty
    When talk mira good evening is spoken with no human
    Then the refusal is LIVE_ONLY
    And state/log/session.jsonl does not exist
    When the speech shard is listening
    And the speech player connects and perceives
    Then presence humans is 1
    When the player talks to mira good evening
    Then the player stdout contains the mira good evening line
    And the player stderr does not contain NOT_IN_SPEC
    And the player stderr does not contain LIVE_ONLY
    And session.jsonl is the mira good evening tape line
    And now.yaml matches the seated snapshot
    And the pack has no tick file
    And the speech processes stop

  Scenario: Speech torn tape tail is not a record
    Given a temp copy of pack packs/oneshot-docks
    When session.jsonl ends with a torn tail
    Then presence humans is 0
    When say hello is spoken on the open shard
    Then the spoken tape starts with seq 1
    And session.jsonl is that spoken tape
    And session.jsonl does not contain torn
    And state_rev is 0
    And world_time matches the shard at the speak
    And the pack has no tick file
