# ADR 0009: v0.1 gate order

- Status: Accepted
- Date: 2026-10-02
- Source: `docs/architecture_v0.1.md` §9, lines 439–485

## Context

"Five gates, all proved. The text below is the record of what each gate made true. It does not open a sixth gate. A session that did the work read this section, the proofs that gate names, and the v0 pins those proofs cite. It did not load the rest of `docs/architecture_v0.md`. The proofs stay in §8. This section does not restate them."

"A gate does not prescribe modules or file order. It names what becomes true, and what stays unbuilt so the session does not invent the next gate. The v0 tests §8 lists as unrewritten stay green in every gate. `v01.rs` is rewritten in Gate A and only there."

The gates overlap on the same code. "Gate B and Gate C both edit the player read loop and the shard thread. Gate C and Gate E both edit `tools/call`. Gate D and Gate E both edit the event sent to the GM."

## Decision

"The gates are linear." "Do not implement two gates in parallel. Do not start a gate whose predecessor is not already green."

The "Still absent" lines are what that gate had not built yet. The section "names what becomes true, and what stays unbuilt so the session does not invent the next gate."

Gate A, proofs 1, 2, 3, 4, 5, 13, 14, 15, 16, 17, and 19. "`spook run` listens. Hellos and refusals work. Startup clears a crashed session. Disconnect, a socket error, and keepalive drop a held seat. One shard thread owns `Now`. `v01.rs` is rewritten here."

Still absent at Gate A: "`say`, `talk`, `attempt`, and `wait` return `NOT_IN_SPEC` and commit nothing. After the line `gm`, every `tools/call` returns `NOT_IN_SPEC`. The scheduler still wakes as v0. Do not append a tape. Do not suppress a wake."

Gate B, proofs 6, 7, and 8. Gate A is green. "Player `say` and `talk` append the tape and scroll back. A seated GM is the other connection, not a tool caller. Speech does not tick and does not bump `state_rev`."

Still absent at Gate B: "No round close. `attempt` and `wait` stay `NOT_IN_SPEC`. No GM tool body. `tools/call` stays the Gate A refusal."

Gate C, proofs 10, 11, 18, and 20. Gate B is green. "With no GM, `wait` commits one live tick. With a GM seated, `wait` queues, and `commit_turn` closes the round. A wake while a seat is held does not tick. A commit that advances `world_time` restarts the sleep. A wake earlier than `tick_every_real_seconds` after `last_tick_at` does not tick after the seats drop. `schedule::wake` stays the v0 function for a caller with no live shard. The seat check and the interval check wrap the tick. The tick that runs borrows the shard thread's `Shard`."

Still absent at Gate C: "Only `intent` and `commit_turn` gain bodies. `attempt` stays `NOT_IN_SPEC`. `perceive`, `npc_act`, `patch_entity`, GM `session_open`, and the GM `say` tool stay `NOT_IN_SPEC`."

Gate D, proof 9. Gate C is green. "`attempt <check> <target>` rolls on the tick that applies it. The player sees success or failure. The GM sees the faces. The roll writes no `Now` path."

Still absent at Gate D: "No new effect class. The GM tools Gate C left refused stay refused, except the check event this proof sends."

Gate E, proofs 12 and 21. Gates B and D are green. "`perceive` reaches the handler. `npc_act` returns `LLM_DISABLED`. `patch_entity` and GM `session_open` return `NOT_IN_SPEC`. The GM `say` tool is the same append as the player line. `spook gm --stdio` is the process a model host spawns. `session_status` with `since` is how that model reads the tape when a notification does not arrive."

Still absent at Gate E: "No fact grant. No non-loopback bind. No second GM grammar. No ack cursor."

## Consequences

All five gates are proved. This record is not a scenario. "Do not implement two gates in parallel."

Gate A's line that `say` returns `NOT_IN_SPEC` is what Gate A had not built yet. Gate B states "Player `say` and `talk` append the tape and scroll back." This record does not add a feature that expects the Gate A refusal for `say`.

## No existing proof

None. §9 states "Five gates, all proved" and "The proofs stay in §8. This section does not restate them."

## Superseded

Later gates in `docs/architecture_v0.1.md` §9 replace earlier "Still absent" sentences.

Gate B replaces Gate A's refusal of `say` and `talk`, and it replaces "Do not append a tape" for speech. Gate C replaces Gate A's and Gate B's refusal of `wait`, and it replaces "No round close" with `commit_turn`. Gate D replaces the refusal of `attempt`. Gate C replaces part of "every `tools/call` returns `NOT_IN_SPEC`" for `intent` and `commit_turn`. Gate E replaces the Gate C refusal of `perceive`, `npc_act`, and the GM `say` tool, and it replaces Gate D's "the GM tools Gate C left refused stay refused" for those tools. Gate E still states "`patch_entity` and GM `session_open` return `NOT_IN_SPEC`."

Gate E's own "Still absent" lines stay in force inside §9: "No fact grant. No non-loopback bind. No second GM grammar. No ack cursor."
