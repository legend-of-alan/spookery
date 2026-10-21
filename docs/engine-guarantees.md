# Engine guarantees

This page states what the engine promises. The feature files test those
promises. Where this page and the feature files disagree, the feature files win.

A tick is a unit of game time. A round is one batch of queued moves, settled
together.

Field names are in the [pack reference](pack-reference.md). Socket bytes are in
the [seat protocol](seat-protocol.md).

## One writer

The journal changes when a round commits, and when migrate stores a new hash of
the world, cast, and rules files. Sitting or leaving writes nothing. A GM say is
speech. A commit runs the queued moves as one round, in queue order. With no GM,
each player line commits its own round. A call off the GM menu is refused.
Speech goes on the tape and leaves the journal and the clock alone. A world,
cast, or rules file that no longer matches the save is refused, and that round
writes nothing, until migrate stores the new hash. An ordinary commit appends
one bundle and flushes that file once. The first record in a new bundle also
flushes the journal directory. `save.yaml` is written at a checkpoint, not on
every commit. A torn last line in the live bundle is cut off on startup, and a
leftover transaction directory is removed. The save on disk stays.

Tested in `features/seats/one_gm_and_three_players.feature`,
`features/rounds/a_line_waits_for_the_gm.feature`,
`features/host/the_host_sits_the_gm_seat.feature`,
`features/speech/speech_appends_and_does_not_tick.feature`,
`features/commit/a_hand_edit_waits_for_migrate.feature`, and
`features/commit/a_transaction_does_not_leak_a_partial_tick.feature`.

## Replay

The journal is a hash chain of bundle files. Each record's hash covers its own
bytes and the previous record's hash. Closed bundles stay, gzip-compressed. A
plain bundle whose checkpoint matches `save.yaml` is closed on the next open
or the next commit, and the next round starts a new bundle. If `save.yaml` or
the gzip does not land, the round stays committed and the host prints a
warning. The next commit publishes that checkpoint and then starts a new
bundle. A plain copy beside a finished gzip is removed. `save.yaml` is the latest
checkpoint, taken every `checkpoint_every`
ticks (100 when the pack omits it). Between checkpoints the current state is the
journal replayed from that snapshot. A missing snapshot is rebuilt from the
bundles. History is not deleted.

Full verification (`spook verify`) replays every bundle from tick 0. Those
bundles replay at the same time, each from the checkpoint it starts at, and the
tip matches one serial replay. Quick verification (`spook verify --quick`)
checks the chain and replays only the latest bundle from the snapshot it starts
at. Startup uses that same snapshot and the live bundle, and does not open a
closed bundle. Verify names the record and the first save line that
differs when a logged move no longer matches. An edit in a closed bundle fails
verification and names that bundle. When logged effects differ from a fresh
run, verify names that round and the first effect that differs. Each journal
record reads back as the bytes that were written. Replay prints the ending
revision and leaves the pack unchanged. Each roll keeps its own dice. An
unrelated roll in the same round does not move them, and verify reproduces
them. A number in a rule can roll dice, including an NPC habit's effect. Those faces are seeded with the
round, written on the journal line, and shown to the GM. Replay rolls them
again. A condition's value does not roll; dice there fail the pack check. A change of who is present, and a migrate between rounds, verify too.

Tested in `features/replay/the_journal_verifies.feature`,
`features/replay/typed_journal_records.feature`,
`features/replay/journal_bundles.feature`,
`features/round/each_roll_has_its_own_seed.feature`,
`features/rules/rule_versions.feature`, and
`features/rules/rule_numbers.feature`.

## Visibility

A player sees a hidden fact only when that character knows it. Stripping prose
does not reveal a hidden sentence. Check flags a hidden sentence copied into a
place body or a note. Audit fails when a player's view contains a hidden fact
they do not know, or a note whose id is that fact. Saying a hidden fact does not
teach it. Audit records a note and still passes. Strict audit fails. Roll faces
go to the GM. Expression dice do too. The player hears success, partial, or miss, and does not see those faces. An NPC's roll sends
the player no roll line. Host status shows the host-only trace. The player's
resume leaves that trace out. The resume digest shows only notes the view shows. A place lists only its own stock.

Tested in `features/seats/one_gm_and_three_players.feature`,
`features/firewall/a_hidden_fact_stays_hidden.feature`,
`features/firewall/spook_audit_proves_no_leak.feature`,
`features/check/spook_check_explains_a_broken_pack.feature`,
`features/checks/the_player_sees_success_and_the_gm_sees_faces.feature`,
`features/checks/an_npc_rolls_its_own_check.feature`,
`features/digest/status_is_the_host_view.feature`, and
`features/digest/resume_follows_the_view.feature`.

## Ordering

A later move sees what an earlier move in that round already changed. One
refused effect rolls the whole rule back. A rule over the effect cap is refused
whole, not cut short, and the log records the refusal. A refused roll is not
reported as a success. A place's verb rule beats the shared rule. A wielded
weapon's attack beats the shared attack. With nothing wielded, the shared
attack runs. Typing a full rule id is refused and the save stays put. A note
addressed to someone present runs before an unclaimed wait, even if the wait
was queued first. An NPC habit stays quiet while a player shares the room. The
GM can still queue it.

Tested in `features/round/a_rules_effects_are_all_or_nothing.feature`,
`features/rules/rules_live_with_their_owner.feature`,
`features/items/items_are_real.feature`, and
`features/stock/the_shop_restocks_and_a_busy_npc_skips.feature`.

## Triggers

A change rule fires in the same round that makes its watch true, and not again
while the watch stays true. Arrival fires a rule on that place, including an
author teleport. A meter that crosses the watched line can set a flag in that
round. An arrival that also meets a flag can grant a fact and start a clock in
that round. Two rules that set each other's flags stop at depth 4, and the round
records the stop. Check warns when change rules set each other's watched
values. A narrate or note on a change rule that can reach a viewer who cannot
see that change is rejected. A change rule may roll. Verify reproduces that
roll, and it passes after a round that fired a change rule.

A narrate line reaches each person it names, unless the line would show a fact
that person is not allowed to know. Someone in another room does not hear a line
addressed to the room. Audit fails when a narrate line repeats a hidden fact that
person does not know. Reveal gives the person it names a fact, and a hidden exit
that fact uncovers then shows up for them. A world flag can be set, and a later
rule sees it for a person who is present. Each viewer gets their own change
summary. The GM sees every change. Tick counters stay with the GM. A learned
fact is told only to that person. An item change in a place is seen by the
players there. A message is seen only by the people allowed to see it. A clock
change is seen by that clock's owners. Entering and leaving are seen by the
players in the room left, the players in the room entered, and the mover. A
world flag is seen by every player. Another person's flag, meter, busy status,
items, or wielded weapon is seen by the players in the same place. A player
sees a change to their own flag. Verify passes after rounds that fired these
rules.

Tested in `features/rules/triggers.feature` and
`features/rules/trigger_delivery.feature`.

## Travel

A missing exit and a locked exit change nothing. A locked exit shows its locked
text. A hidden exit the traveller does not know stays off perceive and help.
The traveller stays put, and stays busy, until the arrival minute. A move before
then is refused. The
round does not shrink: a seated round still advances five minutes, a two-minute
exit is recorded at its minute inside that round, and the next move is the
following round. An empty table advances sixty minutes a round. An earlier
arrival is applied first. When every seated player is busy and no GM is seated,
rounds run ahead without waiting on the wall clock, up to the pack's cap, and
the rest waits for the next action. With a GM seated, nothing advances until the
GM commits.

Tested in `features/map/travel_follows_the_map.feature` and
`features/map/travel_follows_an_exit.feature`.

## Rules version

The save carries the engine's rules version. `spook run` and `spook verify`
refuse a different version. The refusal prints `RULES_REV` and says to start a
fresh save. Saves and journals from older rules are refused, not migrated. A
journal that still holds `presence-NNNN.json` records is refused the same way.

Tested in `features/rules/rule_versions.feature`.

## Failure codes

A refusal names one code, and the refused change is not kept.

`CANT_AFFORD` names the meter the actor cannot pay, and the clock stays put.
`MISSING_ITEM` is using an item the actor does not hold, taking one that is not
there, or dropping one they do not hold. `OUT_OF_RANGE` is a removal that would
pass below zero from a count above zero, and the rest of that rule is not kept.
`NOT_A_WEAPON` is wielding an item that is not a weapon. `UNKNOWN_MOVE` is a
move the actor does not have. `SESSION_FULL` is a fourth player or a second GM.
`SEAT_TAKEN` is a hello for a character another seat holds. `LINE_TOO_LONG`
closes the connection. `NOT_IN_SPEC` is a full rule id used as a move, a GM
hello on the game port, an address that is not loopback, or a verb no rule
matches, which is refused before the round so the turn is kept. `ILLEGAL_CLASS` is
a journal effect the engine does not run. `INELIGIBLE` is a second wait that the
first one makes ineligible. `NO_EXIT` is a missing exit. `BUSY` is a move while
busy.

Check's codes for a broken pack are `SCHEMA`, `NOT_IN_SPEC`,
`PACK_REV_MISMATCH`, `HIDDEN_FACT_IN_BODY`, `HIDDEN_FACT_IN_TEXT`,
`TRIGGER_LEAK`, and `WARNING`. A warning alone does not fail the check.

Tested in `features/rules/rules_live_with_their_owner.feature`,
`features/items/items_are_real.feature`,
`features/round/a_rules_effects_are_all_or_nothing.feature`,
`features/host/a_human_gm_runs_a_round.feature`,
`features/seats/one_gm_and_three_players.feature`,
`features/limits/the_listener_bounds_input.feature`,
`features/loopback/only_loopback_binds.feature`,
`features/replay/typed_journal_records.feature`,
`features/map/travel_follows_the_map.feature`,
`features/check/spook_check_explains_a_broken_pack.feature`, and
`features/rules/rule_versions.feature`.

## Bounds

The table binds loopback only. Any other host is refused before the pack opens,
and the pack is unchanged. A player line with no end, a hello that never
finishes, and an HTTP body over the size cap are refused. The save stays, and
another player can still sit. Invalid text from a seated player is refused, and
that player can still move. A flood from one player does not stop another's
round. A player who stops reading is dropped, and the other keeps receiving
frames. The model client does not dial when a setting is missing.

Tested in `features/loopback/only_loopback_binds.feature`,
`features/limits/the_listener_bounds_input.feature`, and
`features/host/the_host_sits_the_gm_seat.feature`.

## The place

A player arriving in a place sees its display name, the prose they are allowed
to read, the display names of who is there, and the visible exits with travel
time. The same view lists that player's own meters. A meter marked hidden stays
off it, and so does anyone else's. After a round, the player sees one line of
what changed, built from the change records they are allowed to know. That line
names a meter change and an item gained or lost. A hidden fact stays off the
line. A hidden exit stays off the exit list. `help` lists the verbs whose `if`
holds and those exits, and it does not take a round. `look` reprints the prose
and the exits, and it does not take a round. A traveller who has not arrived
sees where the journey ends, as `on the stairs until 21:40`. A player names only
what they hold or what that view shows. Anything else reads like an unknown
word and takes no round. A pack that omits `display_name` fails check. An
unknown word tells the player to type help.

Tested in `features/seats/a_player_knows_the_place.feature`,
`features/seats/actions_visibly_matter.feature` and
`features/items/items_are_real.feature`.

## What the engine does not do

Speech does not grant a fact. A second GM is refused, and so is a fourth player.
The table does not open on a public address. A call that is not on the GM menu
is refused, so a seat cannot patch the save by naming a tool. A player who stops
reading is dropped, and the other seats keep going.

Tested in `features/firewall/spook_audit_proves_no_leak.feature`,
`features/seats/one_gm_and_three_players.feature`,
`features/loopback/only_loopback_binds.feature`,
`features/host/the_host_sits_the_gm_seat.feature`, and
`features/limits/the_listener_bounds_input.feature`.
