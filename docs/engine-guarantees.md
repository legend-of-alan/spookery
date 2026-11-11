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
writes nothing, until migrate stores the new hash. `save.yaml` is written at a
checkpoint, not on every commit. One process writes a pack at a time.
`spook run`, `spook round` and `spook migrate` each hold `state/.pack.lock` for
as long as they have the pack open, and a second one is refused with
`COMMIT_CONFLICT` and writes nothing. Crash recovery runs when a writer takes
that lock: a torn last line in the live bundle is cut off, and a stray temp file
is removed.
The save on disk stays. A write also holds `state/.pack.rw.lock` while it
publishes. `spook status` only reads. It holds a shared lock on that file
while it builds the world and reads the journal, so it waits out a write in
progress and never sees a round or checkpoint halfway published. It does not
take `state/.pack.lock`.
Apart from creating an empty `state/.pack.rw.lock` when it is missing, it
writes nothing under `state/`, and it leaves a running host's commit alone.
`COMMIT_CONFLICT` means only that the lock is held. A disk that refuses a write,
a flush, a rename or a read is `STORE_IO`, and a journal record that no longer
follows its chain is `JOURNAL_DIVERGED`. The seat that sent the line gets that
code.

Tested in `features/seats/one_gm_and_three_players.feature`,
`features/rounds/a_line_waits_for_the_gm.feature`,
`features/host/the_ai_gm_connects_as_the_gm.feature`,
`features/speech/speech_appends_and_does_not_play_a_round.feature`,
`features/commit/a_hand_edit_waits_for_migrate.feature`,
`features/commit/a_crash_leaves_nothing_half_written.feature`,
`features/commit/status_reads_between_writes.feature`,
`features/host/one_writer_per_pack.feature`,
`features/host/store_failures_name_their_reason.feature`, and
`features/replay/save_reads_name_their_reason.feature`.

## Replay

The journal is a hash chain of bundle files. Each record's hash covers its own
bytes and the previous record's hash. Closed bundles stay, gzip-compressed. A
plain bundle whose checkpoint matches `save.yaml` is closed on the next open
or the next commit, and the next round starts a new bundle. If `save.yaml` or
the gzip does not land, the round stays committed and the host prints a
warning. The next commit publishes that checkpoint and then starts a new
bundle. A plain copy beside a finished gzip is removed. `save.yaml` is the latest
checkpoint, taken every `checkpoint_every` rounds (100 when the pack omits it;
the scenario "A pack that does not set checkpoint_every checkpoints every 100
rounds" in `features/replay/journal_bundles.feature` pins it). Between
checkpoints the current state is the journal replayed from that snapshot. A
missing snapshot is rebuilt from the bundles. History is not deleted.

Full verification (`spook verify`) replays the journal once from round 0 and
checks every closed bundle's checkpoint snapshot against its mark. Quick
verification (`spook verify --quick`)
checks the chain and replays only the latest bundle from the snapshot it starts
at. Startup uses that same snapshot and the live bundle, and does not open a
closed bundle. Verify names the record and the first save line that
differs when a logged move no longer matches. An edit in a closed bundle fails
verification and names that bundle. When logged effects differ from a fresh
run, verify names that round and the first effect that differs. Each journal
record reads back as the bytes that were written. Verify leaves the pack
unchanged. Each roll keeps its own dice. An
unrelated roll in the same round does not move them, and verify reproduces
them. A number in a rule can roll dice, including an NPC habit's effect. Those faces are seeded with the
round, written on the journal line, and shown to the GM. `spook verify` replays
the journal, rolls them again from that seed, and checks the save. A condition's value does not roll; dice there fail the pack check. A change of who is present, and a migrate between rounds, verify too.
A pack author can edit the pack's world, cast, or rules while a save exists,
and migrate fits that save to the new canon.
Migrate may carry a hand edit of `save.yaml`. It writes a checkpoint of the
migrated save, replay restarts from that checkpoint, and full verification
trusts the edit and names it as `operator edit trusted at state_rev N`. A hand
edit made while the journal has rounds past `save.yaml` is refused with
`HAND_EDIT`, and migrate writes nothing. Opening a pack whose journal does not
replay to its recorded hash is refused with `JOURNAL_DIVERGED`.

Tested in `features/replay/the_journal_verifies.feature`,
`features/commit/a_hand_edit_waits_for_migrate.feature`,
`features/meters/modifiers_stack_and_expire.feature`,
`features/replay/typed_journal_records.feature`,
`features/replay/journal_bundles.feature`,
`features/round/each_roll_has_its_own_seed.feature`,
`features/rules/rule_versions.feature`, and
`features/rules/rule_numbers.feature`.

## Visibility

A player sees a hidden fact only when that character knows it. Stripping prose
does not reveal a hidden sentence. Saying a hidden fact does not
teach it. Roll faces
go to the GM. Expression dice do too. The player hears success, partial, or miss, and does not see those faces. An NPC's roll sends
the player no roll line. Host status shows the host-only trace. The player's
resume leaves that trace out. The resume digest shows only notes the view shows. A place lists only its own stock.

Tested in `features/seats/one_gm_and_three_players.feature`,
`features/visibility/a_hidden_fact_stays_hidden.feature`,
`features/rolls/the_player_sees_success_and_the_gm_sees_faces.feature`,
`features/rolls/an_npc_rolls_its_own_check.feature`,
`features/digest/status_is_the_host_view.feature`, and
`features/digest/resume_follows_the_view.feature`.

## Ordering

A later move sees what an earlier move in that round already changed. One
refused effect rolls the whole rule back. A rule over the effect cap is refused
whole, not cut short, and the log records the refusal. That holds for a change
rule too. A refused roll is not reported as a success. A roll target that
fails to evaluate refuses that move with its error, and the rest of the round
lands and commits. A move pays the cost of the rule it rolled for, even when an
earlier move that round changed which rule its verb would pick. A place's verb rule beats the shared rule. A cast
member's verb rule is picked by naming them while they share the actor's place,
and a bare verb never reaches it. A wielded item's rule for any verb beats the
target's, the place's and the shared rule, so a wielded weapon's attack beats the
shared attack. With nothing wielded, the shared attack runs. Typing a full rule id is
refused and the save stays put. A note addressed to someone present runs before
an unclaimed wait, even if the wait was queued first. So does a move through an
exit into a seated player's room. An NPC habit stays quiet while a player shares
the room. The GM can still queue it, and a GM-queued habit is checked the same
as one the engine runs, so a habit that sets a flag its cast file does not
list is refused `UNKNOWN_ID` either way. A move the round has no room for is
refused `BUDGET`, never dropped: one past the intent limit, and every one after
a rule that goes over the state-key limit.

Tested in `features/round/a_rules_effects_are_all_or_nothing.feature`,
`features/round/an_intent_the_round_has_no_room_for_is_refused_budget.feature`,
`features/round/a_broken_roll_threshold_refuses_one_move.feature`,
`features/round/apply_pays_the_rule_that_rolled.feature`,
`features/rules/rules_live_with_their_owner.feature`,
`features/rules/a_gm_queued_habit_is_judged_like_the_engines.feature`,
`features/items/items_are_real.feature`, and
`features/habits/the_shop_restocks_and_a_busy_npc_skips.feature`.

## Triggers

A change rule fires in the same round that makes its watch true, and not again
while the watch stays true. Change rules run right after each action starts and
after each one finishes, so a door opened twice in one round fires its watcher
twice, and a rule can refuse a later action in that round. The depth limit
counts the passes after each action, not the round. Arrival fires a rule on that place, including an
author teleport. A meter that crosses the watched line can set a flag in that
round. An arrival that also meets a flag can grant a fact and move a world
meter in that round. Two rules that set each other's flags stop at depth 4, and the round
records the stop. Check warns when change rules set each other's watched
values. A narrate or note on a change rule that can reach a viewer who cannot
see that change is rejected. A change rule may roll. Verify reproduces that
roll, and it passes after a round that fired a change rule. A world flag or world
meter change fires each watching rule once, whoever is seated, with no one as
its subject, so check refuses `self` and `here` in that rule. Its narrate with
no `to` reaches every player present.

A narrate line reaches each person it names, unless the line would show a fact
that person is not allowed to know. Someone in another room does not hear a line
addressed to the room. Reveal gives the person it names a fact, and a hidden exit
that fact uncovers then shows up for them. A world flag can be set, and a rule
that watches it fires once. Each viewer gets their own change
summary. The GM sees every change. Tick counters stay with the GM. A learned
fact is told only to that person. An item change in a place is seen by the
players there. A message is seen only by the people allowed to see it.
Entering and leaving are seen by the players in the room left, the players in
the room entered, and the mover. Another person's busy status, items, or
wielded weapon is seen by the players in the same place. No player sees a flag
change, their own or the world's. Verify passes after rounds that fired these
rules.

Tested in `features/rules/triggers.feature`,
`features/rules/change_rules_fire_after_each_action.feature`,
`features/rules/trigger_delivery.feature`, and
`features/rules/a_world_change_rule_runs_once.feature`.

## Meters

A meter lives on the entity whose file declares it: every actor for
`rules/meters.yaml`, one actor for a cast file's `meters`, and the world for
`world/meters.yaml`. A world meter is held once, so every rule that names
`of: world` reads and writes the same value, and its change rules fire once per
change, whoever is seated. The world and an actor may each carry a meter of one name, and
`of` picks which. Check refuses a cast meter whose id `rules/meters.yaml` or
another cast file already has, and a rule whose fixed `of` names a meter that
entity does not carry.

An actor's meter change is told to that actor only, not to a player in the
same room. A world meter is hidden from players unless the pack shows it. A
shown one is told to every player, and its current value is in each view, so a
player who comes back sees it. The GM sees every meter change. A meter never
pauses: a rule that should wait for a seated player says so in its `if`.

Tested in `features/meters/meters_belong_to_their_entity.feature` and
`features/meters/the_debt_waits_while_no_player_is_seated.feature`.

## Modifiers

A modifier adds the sum of its stacks to one actor meter and writes nothing to
the meter's offset. Each application is one stack with its own magnitude, up to
the modifier's `max_stacks`, which is 1 when the pack leaves it out. At the cap,
an application whose absolute value is larger than the weakest stack replaces
it, and a weaker or equal one changes no stack. The modifier has one clock.
Every application, a tie included, sets the end to now plus the smaller of the
ticks left plus its `ticks` and the longest `ticks` applied so far, this one
included. When the world reaches the end, the modifier expires and every stack
goes at once. Different modifiers on one meter add up. A player sees a modifier
change on their own actor only, and not one the pack hides. Replay gives the
same journal, and migrate drops a modifier the pack no longer defines or now
defines on another meter. Check refuses a `ticks` or `max_stacks` below 1, a
modifier of a meter no actor carries, and an effect that names a modifier the
pack does not define.

Tested in `features/meters/modifiers_stack_and_expire.feature`.

## Travel

A missing exit and a locked exit change nothing. A locked exit shows its locked
text. An exit with an `if` is locked until, at the end of a round, some actor in
its place meets that `if`. Then the engine records an `exit_open` effect that sets the
exit's `open`, and the exit stays open for every actor until a rule sets `open`
to false. A hidden exit the traveller does not know stays off the view and `help`.
A walk that takes ticks puts the traveller on the exit at once, in neither
room, and keeps them busy until the arrival tick. A move before then is
refused. The
round does not shrink: a seated round still advances five ticks, a two-tick
exit is recorded at its tick inside that round, and the next move is the
following round. An empty table advances sixty ticks a round. An earlier
arrival is applied first. When every seated player is busy and no GM is seated,
rounds run ahead without waiting on the wall clock, up to the pack's cap, and
the rest waits for the next action. With a GM seated, nothing advances until the
GM commits.

Tested in `features/travel/travel_follows_the_map.feature`,
`features/travel/travel_follows_an_exit.feature` and
`features/travel/exit_locks_open_once_and_stay_open.feature`.

## Actions

Every move but `say` starts and finishes through one path, whether it travels,
teleports or does anything else. A move is checked when it is queued, by a
player or by the GM, so a busy actor is refused `BUSY` at once and keeps the
turn. It is checked again when it starts, since an earlier move that round may
have changed the world, and a habit is checked only then. A GM-queued habit is
checked the same as one the engine picks, its `if` included. The start pays
its cost and, unless the rule says otherwise, uses the actor's act. A move
that takes no ticks finishes in the round it starts. A timed one finishes in
the round that reaches its tick, and every effect lands then, a teleport
included. After the finish the actor stays busy through the rule's cooldown.
A player's view says what keeps them busy and until when.

Tested in `features/rules/one_action_lifecycle.feature`.

## Cancelling an action

A player may cancel their own timed action while it is under way, when its rule
says `cancellable`. The cancel is a move like any other, played in a round, so
replay sees it. The action is dropped and its actor is free at once. Nothing it
cost comes back, and its finish never runs. A cancel with no action under way,
on a rule that is not cancellable, or on a walk is refused `INELIGIBLE`, and
the action still lands. No one cancels another's action.

Tested in `features/rules/cancel_own_action.feature`.

## Game time

World time is a count of ticks, and the engine knows no day or hour. A pack may
name cycles in its budget, each with a length in ticks. A `cycle` condition
reads the tick within its named cycle, world time modulo the length, and may
wrap past the cycle's end. Idle and live acts each have a cap per window of
ticks, and a round that crosses the end of a window gives that count back.
Players and the GM see plain ticks, as `until tick 1300`. Check refuses a cycle
length or an act window of 0, a `cycle` that names no cycle in the budget, or a
bound at or past the cycle's length.

Tested in `features/rules/game_time_counts_in_ticks.feature`,
`features/seats/a_player_knows_the_place.feature`, and
`features/check/spook_check_explains_a_broken_pack.feature`.

## Rules version

The save carries the engine's rules version. `spook run` and `spook verify`
refuse a different version. The refusal prints `RULES_REV_MISMATCH` and says to start a
fresh save. Saves and journals from older rules are refused, not migrated.

Tested in `features/rules/rule_versions.feature`.

## Failure codes

A refusal names one code, and the refused change is not kept.

`CANT_AFFORD` names the meter the actor cannot pay, and the clock stays put.
`MISSING_ITEM` is using an item the actor does not hold, taking one that is not
there, or dropping one they do not hold. `OUT_OF_RANGE` is a removal that would
pass below zero from a count above zero, and the rest of that rule is not kept.
`NOT_A_WEAPON` is wielding an item that is not a weapon. `UNKNOWN_MOVE` is a
move the actor does not have. `TABLE_FULL` is a fourth player or a second GM.
`SEAT_TAKEN` is a claim for a character another seat holds, even one whose
connection dropped. `ACTOR_MISMATCH` is a GM move for a PC a player holds.
`UNKNOWN_ID` is a claim by a token no claim holds. `LINE_TOO_LONG`
is a message over 8 KiB, and the connection stays up. `SCHEMA` is a command the
hello did not list. `NOT_IN_SPEC` is a full rule id used as a move, an address
that is not loopback, or a verb no rule matches, which is refused before the
round so the turn is kept. `UNKNOWN_EFFECT` is
a journal effect the engine does not run. `INELIGIBLE` is an NPC's habit in a round
where a move the GM queued for that NPC already landed. `NO_EXIT` is a missing exit.
`BUSY` is a move while busy.

Check's codes for a broken pack are `SCHEMA`, `NOT_IN_SPEC`,
`PACK_REV_MISMATCH`, `TRIGGER_LEAK`, and `WARNING`. A warning alone does not
fail the check.

Tested in `features/rules/rules_live_with_their_owner.feature`,
`features/items/items_are_real.feature`,
`features/round/a_rules_effects_are_all_or_nothing.feature`,
`features/host/a_human_gm_runs_a_round.feature`,
`features/seats/one_gm_and_three_players.feature`,
`features/seats/a_claim_comes_back_with_its_token.feature`,
`features/limits/the_listener_bounds_input.feature`,
`features/loopback/only_loopback_binds.feature`,
`features/replay/typed_journal_records.feature`,
`features/travel/travel_follows_the_map.feature`,
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
`features/host/the_ai_gm_connects_as_the_gm.feature`.

## The place

A player arriving in a place sees its display name, the prose they are allowed
to read, the display names of who is there, and the visible exits with travel
time. The same view lists that player's own meters. A meter marked hidden stays
off it, and so does anyone else's. After a round, the player sees one line of
what changed, built from the change records they are allowed to know. That line
names a meter change and an item gained or lost. A hidden fact stays off the
line. A hidden exit stays off the exit list. `look` answers with a fresh view and
does not take a round, and nor do `help` and `map`. The table sends both as
data and the client prints them: `help` lists the commands this player can use
here and now, each with what it takes and what they can name for it, and `map`
lists each place their save says they visited, with the exits they saw there
when they last stood there at the end of a round. Where those exits go and
whether they are locked is read now. A busy player sees what keeps them busy:
`on the stairs until tick 1300` on a walk, `chanting until tick 1300` on any
other timed move, and `recovering until tick 1300` in a cooldown. A player names only
what they hold or what that view shows. Anything else reads like an unknown
word and takes no round. A pack that omits `display_name` fails check. An
unknown command tells the player to type help.

Tested in `features/seats/a_player_knows_the_place.feature`,
`features/changes/actions_visibly_matter.feature`,
`features/items/items_are_real.feature`,
`features/perceive/the_map_shows_exits_as_the_actor_last_saw_them.feature` and
`features/check/spook_check_explains_a_broken_pack.feature`.

## What the engine does not do

Speech does not grant a fact. A second GM is refused, and so is a fourth player.
The table does not open on a public address. A call that is not on the GM menu
is refused, so a seat cannot patch the save by naming a tool. A player who stops
reading is dropped, and the other seats keep going.

Tested in `features/speech/speech_appends_and_does_not_play_a_round.feature`,
`features/seats/one_gm_and_three_players.feature`,
`features/seats/a_claim_comes_back_with_its_token.feature`,
`features/loopback/only_loopback_binds.feature`,
`features/host/the_ai_gm_connects_as_the_gm.feature`, and
`features/limits/the_listener_bounds_input.feature`.
