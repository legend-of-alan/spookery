# Shard Tick Spec — v0.3

Status: Frozen for v0 implementation. Naming pass only vs v0.2 (pack / `spook` / `pack_rev`).  
Scope: invite-only persistent pack worlds for 2–16 humans  
Depends on: pack format (text canon + `state/`), knowledge firewall, table MCP tools, `NOW_AND_EFFECTS.md`, `BRANDING.md`

**v0.3 delta:** Packetry terms replaced. Mechanics unchanged.

**v0.2 delta:** Draft stamp removed. `llm_npc_intents_per_tick` default is **0** everywhere. Kernel intent derivation is a deterministic function (§4.3). Eligible-NPC filter is a boolean predicate, not an expression language (§14). Effect payloads live in `NOW_AND_EFFECTS.md`.

This spec defines how a shard continues when nobody is typed in, and how a tick
process is allowed to touch the world. It is a runtime contract, not a game design
doc. Games choose *which* clocks and *which* moves to enable. The engine enforces
the limits below.

---

## 0. One-sentence rule

Humans and NPC agents submit **intents**.  
Only the **tick** applies intents and writes `state/`.  
No model, player client, or NPC process may patch canon or state directly.

---

## 1. Goals and non-goals

### Goals

- A friends’ world stays *alive* while they sleep: clocks advance, relationships
  drift, rumors move, unfinished work progresses.
- Offline behavior is **bounded, auditable, and reversible**.
- Live play and offline sim share the same kernel and the same effect vocabulary.
- Replay a night of ticks from a log and get the same `state/` (given the same
  seed and the same resolved RNG stream).

### Non-goals

- Spatial MMO movement, aggro tables, interest management.
- Thousand-NPC living-city simulation.
- LLM-authored plot while nobody is watching (no “the BBEG conquered the kingdom
  in chapter form at 3am”).
- Real-time combat resolution in the tick. Combat is a live or scheduled
  **encounter**, not a background job.

---

## 2. Shard vs session

| | Session | Shard |
|---|---|---|
| Lifetime | One sitting | Until the host deletes it |
| Presence | At least one human usually | Zero humans is valid |
| Writer | `commit_turn` inside a sitting | `tick_commit` on a clock |
| Clock | Scene / combat rounds | World time + tick index |
| Save | Optional snapshot | Continuous `state/` + tick log |

A **session** is a presence window on a shard. Opening a session does not stop
the shard clock unless the ruleset says so (see §6.3 pause policy).

Identity:

```text
shard_id        durable
pack_rev      git hash or content hash of immutable canon
state_rev       monotonic integer, +1 per successful tick_commit
tick_index      monotonic integer, +1 per tick attempt that runs the loop
world_time      integer minutes since shard epoch (ruleset-defined epoch)
```

Canon files under `world/`, `cast/`, `rules/` are immutable at runtime.
Ticks may only write:

```text
state/now.yaml          authoritative mutable snapshot
state/log/tick-NNNN.jsonl
state/memories/<id>.jsonl
state/rumors.yaml       optional derived view
saves/<name>.yaml       explicit snapshots
```

If a tick believes canon must change (an NPC dies permanently, a building burns),
it writes a **canon proposal** into `state/proposals/` and sets a flag. A human
or studio agent accepts it later with `patch_entity`. Ticks never edit `cast/`
or `world/` themselves.

---

## 3. Clock

### 3.1 Two clocks

- **World clock** — fictional time (`world_time`). Used in narration and
  schedules (“Mira closes the shop at dusk”).
- **Tick clock** — real scheduling. The host configures `tick_every_real_seconds`.

Default mapping for a friends shard:

| Real time | World time advanced | Notes |
|---|---|---|
| 1 tick while humans present | ruleset `live_minutes_per_tick` (default 5) | stay close to play |
| 1 tick while shard empty | ruleset `idle_minutes_per_tick` (default 60) | compress downtime |
| Max world-hours per real-hour idle | `idle_max_world_hours_per_real_hour` (default 8) | stop time-lapse abuse |

A shard that has been idle 10 real hours does **not** simulate 10 hours of
dense NPC life. It simulates a capped number of ticks (§5).

### 3.2 Tick trigger

A tick runs when any of these fire:

1. Scheduler: `now >= last_tick_at + tick_every_real_seconds`
2. Catch-up: shard wakes after downtime (capped; §5.2)
3. Live flush: a human `commit_turn` may request `include_tick=true` so
   world clocks do not stall during play
4. Manual: `tick_run` MCP tool with `reason=manual` (host / tests only)

Ticks are serialized per shard. No concurrent ticks.

---

## 4. Actors and authority

```text
Player client  -> intent (player)
NPC agent      -> intent (npc)     [optional; often skipped offline]
Tick process   -> collect, filter, resolve, commit
Kernel         -> dice, inventory, clocks, legality
```

### 4.1 Privilege tiers

| Actor | May call | May not |
|---|---|---|
| Player MCP (if exposed) | `intent`, `perceive` as self, `session_*` | `apply_effect`, `tick_*`, raw state |
| GM agent (live session) | table tools in the live spec | `tick_run` except via flush flag |
| Tick process | tools in §8 only | `patch_entity`, canon writes, unscoped `perceive` |
| Studio / host | everything, with audit | — |

### 4.2 Offline NPCs are not full agents by default

Default mode: **profile-driven kernel moves**, no LLM on the idle tick.

Each tick, for each eligible NPC, the kernel derives at most one candidate
intent from structured profile fields (`goals`, `schedule`, `needs`,
`relationships`, `resources`) plus `state/now.yaml`.

Optional mode: **sampled LLM adjudication**. At most
`llm_npc_intents_per_tick` NPCs (default **0**) may have an intent drafted
by a model via `npc_act`. The model still cannot write state.

v0 ships `kernel_only`. Do not turn `npc_act` on until tests 1–8 pass.

If the host cannot afford inference, idle ticks stay kernel-only. The world
still moves.

### 4.3 Kernel intent derivation (deterministic)

`kernel_intents_for_eligible_npcs` is a pure function of canon frontmatter + `Now`.  
It emits **at most one** intent per NPC per tick. No model. No hidden randomness except the §5.4 stable shuffle for *priority among extras*, which runs after this function.

**Eligible (v0 predicate, not an expression language):**

```text
tick.eligible == true
AND NOT now.actors[id].busy
AND (tick.tier in {named, major}
     OR tick.plot_relevant == true
     OR tick.schedule is non-empty)
AND idle_acts_today < idle_acts_per_world_day   # when presence.humans == 0
```

Missing `tick` block ⇒ inert while empty. No string DSL.

**First match wins:**

1. `pay_upkeep` — a listed `needs` key is at or below its floor and `pay_upkeep` is legal.
2. `restock` — actor is at a place they own stock for, `stock_rate[sku]` is due this tick, result would stay ≥ 0.
3. `travel` — current `world_hour` schedule place ≠ `now.actors[id].place`, destination is in `range`.
4. Schedule `move` for this hour (`work` or `rest`) if already in the scheduled place.
5. `drift_relationship` — a declared pair has `stance != target`, last interaction ≥ 1 world-day ago, neither busy.
6. `spread_rumor` — a rumor with `heat >= 1` is known by this actor, an adjacent recipient (same place or declared social edge) is firewall-allowed, hop budget remains.
7. `leave_note` — profile or clock says a player-facing note is due.
8. `advance_clock` — a clock this actor owns is not paused and is due.
9. Else no intent (`wait` is not emitted; `wait` does not count against `max_npcs_acting`).

Move → effect map: `NOW_AND_EFFECTS.md` §4. Illegal knowledge, out-of-range travel, paused clocks, and unknown keys fail closed at `apply_effect`, not by inventing a different move.

---

## 5. Budgets (the heart of the spec)

Budgets are per tick unless noted. Exceeding a budget drops the excess
intents with `rejected: budget`.

### 5.1 Per-tick global caps

| Budget | Default | Purpose |
|---|---|---|
| `max_intents_applied` | 12 | hard stop on world churn |
| `max_npcs_acting` | 8 | not every villager every tick |
| `llm_npc_intents_per_tick` | 0 | cost + chaos cap. v0 is kernel_only |
| `max_effects_per_intent` | 4 | no kitchen-sink moves |
| `max_state_keys_mutated` | 24 | diff stays reviewable |
| `max_memory_writes` | 8 | memory log does not explode |
| `max_rumor_hops` | 2 | gossip does not cover the map |
| `max_world_minutes` | from clock table | time itself is budgeted |

### 5.2 Catch-up cap

On wake after downtime of `D` real hours:

```text
ticks_to_run = min(
  ceil(D * 60 / tick_every_real_seconds),
  idle_catchup_max_ticks          # default 12
)
```

Twelve ticks after a week away is a *summary* of absence, not a full simulated
week. If the ruleset needs “a season passed,” that is one **calendar effect**
(`season_advance`), not 2,000 ticks.

### 5.3 Per-NPC rate limits

Tracked in `state/now.yaml` under `actors.<id>.tick`:

| Limit | Default |
|---|---|
| Acts at most once per tick | always |
| `idle_acts_per_world_day` | 3 |
| `live_acts_per_world_hour` | 6 (only while a session is open) |
| Cannot act if `busy` flag set by a live encounter | — |

### 5.4 Priority when over budget

Sort candidate intents, keep prefix until cap:

1. Intents that resolve a **player-owned clock** (quest timers the party started)
2. Intents targeting a **present player** or their claimed assets
3. Intents from NPCs with `tier` in `{named, major}` or `plot_relevant=true`
4. Intents that would fire a **pending proposal** (siege completes, debt due)
5. Everyone else, stable-shuffled by `hash(shard_id, tick_index, npc_id)`

Extras are not queued forever. They die. Persistent pressure belongs in
**clocks**, not a backlog of 400 intents.

---

## 6. What an offline NPC may change

An offline move is legal only if it is in the allowlist **and** passes the
firewall **and** fits budget.

### 6.1 Allowlist — effect classes

Each applied intent expands to one or more **effects**. Only these classes
are legal when `presence.humans == 0`.

| Class | Examples | Limits |
|---|---|---|
| `clock` | increment a named clock, expire a timer | clocks declared in pack/rules |
| `need` | hunger, fatigue, coin-for-living | clamp to profile `needs` ranges |
| `stock` | shop inventory restock / sell-through | ± declared `stock_rate`, never below 0 |
| `location` | move along a listed exit / schedule | only to places in profile `range` or `schedule` |
| `relationship` | drift toward a declared stance | max ±1 step per world-day per pair |
| `reputation` | local rumor standing | max ±1 per faction per world-day |
| `memory` | append one private memory | no secrets the actor does not know |
| `rumor` | seed or hop a public rumor | payload from known facts only |
| `resource` | spend/gain listed personal resources | cannot mint unlisted items |
| `flag` | set/clear flags named on the profile | no inventing flag keys |
| `proposal` | file a canon proposal | no auto-accept |
| `message` | leave a note for a player / other NPC | queued; not instant scene prose |

### 6.2 Blocklist — never on an idle tick

- Kill, maim, or permanently remove a **player character**
- Destroy or steal a player-owned item unless a pre-declared clock matured
  (`debt_collection_due`) and the pack opted into that rule
- Reveal a `hidden` / `gm_secret` fact into any public channel
- Invent new places, NPCs, items, or factions
- Start or resolve a combat encounter
- Move an NPC out of their `range` / off their `schedule` graph
- Rewrite another actor’s memories
- Change `world/` or `cast/` files
- Grant XP, levels, rare loot
- Teleport players
- Advance main-plot beats tagged `requires_presence: true`

### 6.3 Presence policy

```yaml
pause_policy:
  named_plot_clocks: pause_when_empty      # default
  subsistence_clocks: always_run           # food, shop hours, weather
  relationship_drift: always_run
  travel: run_if_schedule
```

If the party left mid-assassination, the assassination clock **pauses**.
The innkeeper still closes the bar. This is how you avoid “we slept and the
module finished itself.”

### 6.4 Relationship drift formula (default)

For each pair `(a, b)` with a declared relationship and no live scene:

```text
delta = clamp(stance_target(a,b) - current, -1, +1)
apply at most once per world-day
skip if either actor has flag busy
skip if last_interaction_world_time is within 1 world-day
```

`stance_target` comes from the profile (`allies`, `rivals`) and recent
memories (positive/negative tags). No LLM required.

### 6.5 Rumor hop

A rumor is a structured object, not free text the model can escalate.

```yaml
rumor:
  id: r_184
  fact_ids: [fact_ledger_short]     # must exist in pack or state
  origin: mira
  heat: 1                           # 1..3
  known_by: [mira, aldric]
```

On an idle tick a rumor may hop to one adjacent actor (same place **or**
declared social edge) if `heat >= 1` and hop budget remains. The recipient
must be allowed to know that fact under the firewall. If not, the hop
fails closed.

---

## 7. Intent schema

```yaml
intent:
  id: uuid
  tick_index: 1842
  actor_id: mira
  actor_kind: npc          # npc | player | system
  source: kernel           # kernel | llm | player | system
  move: restock            # enum from ruleset
  target: { place: docks_shop }
  args: { sku: lamp_oil, delta: 4 }
  rationale_code: schedule.dusk_close
  # never trusted:
  llm_say: null
```

Legal `move` values are defined in `rules/moves.yaml`. Unknown moves reject.

Minimum idle move set for v0:

```text
wait
travel
work
restock
rest
pay_upkeep
drift_relationship
spread_rumor
leave_note
file_proposal
advance_clock
```

Live sessions may add `talk`, `attempt`, `attack`, `give`, `take`. Those
moves are rejected if `presence.humans == 0`.

---

## 8. MCP tools the tick process may call

The tick process attaches **only** this server profile: `pack-tick`.
Tool list is fixed and ordered.

### 8.1 Allowlist

| Tool | Use in a tick | Notes |
|---|---|---|
| `session_status` | Start of tick | Read presence, pause flags, world_time |
| `perceive` | Before drafting / applying an NPC intent | **Required** `as=npc:<id>`. Tick process must not call `as=gm` |
| `recall` | Optional, same viewer scope | Keyword/semantic over allowed facts only |
| `npc_act` | At most `llm_npc_intents_per_tick` times | Returns a *move*, not prose that mutates |
| `resolve_check` | Only if a move’s rule requires a roll | Seeded RNG from `tick_index` |
| `apply_effect` | Apply a legal effect from §6.1 | Server validates class + firewall |
| `tick_commit` | Once at end | Replaces live `commit_turn` for idle ticks |

### 8.2 Denylist for the tick process

`intent` from this process is unnecessary (it *is* the collector).  
Forbidden: `patch_entity`, `read_entity` of hidden files as GM,
`session_open` as a player, `run_conformance`, any studio tool,
`apply_effect` with class not in §6.1, `perceive` without `as`,
`npc_act` for an NPC already acted this tick.

### 8.3 Tool contracts (tick profile)

**`perceive`**

```text
in:  { as: "npc:mira", focus?: place|actor, budget_tokens?: 800 }
out: {
  viewer, place, visible_actors[],
  known_facts[],          # ids + short text
  unknown_mentioned[],    # names present but not knowable — no bodies
  clocks[], stocks[],
  schedule_hint
}
```

If the model (or kernel) needs a fact not in `known_facts`, it does not
exist for that actor.

**`npc_act`**

```text
in:  { actor_id, perceive_ref, allowed_moves[] }
out: {
  move, target, args,
  public_say?: string,        # flavor only; not applied as canon
  private_goal?: string,
  effect_hints[]              # suggestions; kernel re-validates
}
```

`public_say` is stored on the tick log for later narration. It is **not**
shown to anyone until a human is present, unless `leave_note` was the move.

**`apply_effect`**

```text
in:  { intent_id, class, payload, as: "npc:mira" }
out: { accepted: bool, code?, diff? }
```

`class` + `payload` must match `NOW_AND_EFFECTS.md` §3. Unknown class or extra payload keys fail `ILLEGAL_CLASS`.

Failure codes: `ILLEGAL_CLASS`, `ILLEGAL_KNOWLEDGE`, `OUT_OF_RANGE`,
`BUDGET`, `PAUSED_CLOCK`, `UNKNOWN_KEY`, `NOT_OWNER`.

**`tick_commit`**

```text
in:  {
  tick_index, world_minutes_advanced,
  applied_intent_ids[],
  effects[],                  # already accepted
  memory_writes[],
  proposals[]
}
out: { state_rev, diff_summary, log_path }
```

`tick_commit` is atomic. On failure the shard remains at the previous
`state_rev`. Partial effects from a crashed tick do not linger.

### 8.4 Who calls `npc_act`

| Mode | Tick process behavior |
|---|---|
| `kernel_only` | never calls `npc_act` |
| `sampled_llm` | call for the single highest-priority eligible NPC |
| `live_assist` | not a tick concern; GM agent uses table tools |

---

## 9. Tick loop

Pseudocode the implementation must match in spirit:

```text
lock shard
status = session_status()
world_minutes = pick_advance(status)
candidates = collect_player_queued_intents()
             + kernel_intents_for_eligible_npcs(status)
             + maybe_one_llm_intent()

candidates = filter_blocklist(candidates, status.presence)
candidates = prioritize(candidates)[:max_intents_applied]

applied = []
for intent in candidates:
    view = perceive(as=intent.actor)
    if not legal(intent, view): reject; continue
    if needs_roll(intent): resolve_check(seeded)
    for effect in expand(intent)[:max_effects_per_intent]:
        apply_effect(effect) or reject remainder
    applied.append(intent)

tick_commit(...)
unlock
```

`maybe_one_llm_intent` must `perceive` first and pass `allowed_moves`
drawn from §7 minus live-only moves.

---

## 10. Live play interaction

When a human session is open:

- Tick period may shorten (`live_tick_every_real_seconds`, default 60s)
  or be replaced by `commit_turn(include_tick=true)`.
- NPCs in the same **place** as a player are `busy`. Idle moves for them
  skip; the GM agent owns their behavior.
- Player intents always outrank NPC idle intents in §5.4.
- A tick may still run subsistence clocks in *other* places (the shop
  across town restocks).

Never run a long catch-up burst while humans are present. Finish at most
one catch-up tick, then yield to play.

---

## 11. Memory and narration

Idle ticks write **structured memories**, not scenes.

```yaml
memory:
  actor: mira
  tick_index: 1842
  fact_ids: [fact_sold_four_oil]
  tags: [work, uneventful]
  text: "Closed shop. Sold four lamp oil. Ledger still short."
```

`text` is optional flavor, max 240 chars, must not contain fact-ids the
actor does not know. Conformance tests grep memories for secret strings.

When humans return, the GM agent may call `recall` as a player or as GM
(GM only in a live session) and summarize. The tick process itself does
not generate a “what you missed” novella.

Player-facing resume is viewer-scoped. It may include messages to that
player, rumors that entered their `known_by`, and plot clocks they own.
It must not include off-screen stock, other places’ inventories, or
rumors they do not know. Host `spook status` may list every effect
class for tests. That host view is not a play-door payload.

See product spec §11. This is a perception rule, not a client skin.

---

## 12. Determinism and audit

Every tick writes one JSONL line:

```json
{
  "tick_index": 1842,
  "state_rev_before": 9001,
  "state_rev_after": 9002,
  "world_time_before": 10260,
  "world_time_after": 10320,
  "presence_humans": 0,
  "seed": "shard:abc|tick:1842",
  "intents": [...],
  "rejected": [...],
  "effects": [...],
  "tool_trace": ["session_status", "perceive:mira", "apply_effect", "tick_commit"]
}
```

RNG for `resolve_check` during ticks: `rng = blake2s(seed)`. Replaying
the log with `source=kernel` intents must reproduce `state_rev_after`.
LLM intents are stored verbatim; replay uses the stored move, not a new
model call (`replay_mode=recorded`).

---

## 13. Failure and degradation

| Condition | Behavior |
|---|---|
| LLM timeout / error | skip `npc_act`; finish kernel intents |
| `perceive` empty | actor cannot act; `wait` |
| `tick_commit` conflict | abort tick, keep previous rev |
| Budget exceeded mid-loop | stop applying, commit what passed |
| Pack hash mismatch | refuse ticks until host migrates |
| > `idle_catchup_max_ticks` pending | collapse remainder into one `season_advance` proposal, do not silently drop plot clocks |

If the shard cannot tick (crash loop), presence is still allowed. Live
play must not depend on the scheduler being healthy.

---

## 14. Ruleset hooks (data, not code)

`rules/tick.yaml` example:

```yaml
tick_every_real_seconds: 300
idle_minutes_per_tick: 60
live_minutes_per_tick: 5
idle_catchup_max_ticks: 12
idle_max_world_hours_per_real_hour: 8
max_intents_applied: 12
max_npcs_acting: 8
llm_npc_intents_per_tick: 0          # kernel_only. do not raise in v0
pause_policy:
  named_plot_clocks: pause_when_empty
  subsistence_clocks: always_run
# eligibility is the §4.3 predicate. no expression language in v0.
```

Profiles opt in:

```yaml
# cast/mira.md frontmatter
tick:
  eligible: true
  range: [docks_shop, docks_street, mira_rooms]
  schedule:
    - { world_hour: [6, 18], place: docks_shop, move: work }
    - { world_hour: [18, 6], place: mira_rooms, move: rest }
  needs: [coin, rest]
  stock_rate: { lamp_oil: 4 }
```

Missing `tick` block ⇒ NPC is inert while the shard is empty.

---

## 15. Conformance tests (minimum)

These belong in the engine suite, not as documentation theater.

1. **Closed secret** — idle ticks for 100 steps; `hiddenInfo` string from
   `mira.md` never appears in `aldric` memories, rumors, or `perceive as=npc:aldric`.
2. **No self-win** — plot clock `requires_presence: true` does not increment
   across 12 idle ticks.
3. **Shop lives** — `lamp_oil` stock changes within `stock_rate` and stays ≥ 0.
4. **Catch-up cap** — 72h downtime produces ≤ `idle_catchup_max_ticks` commits.
5. **Busy skip** — player present in `docks_shop`; Mira produces no idle move.
6. **Atomic commit** — kill the process after `apply_effect` before
   `tick_commit`; restart loads previous `state_rev`.
7. **Replay** — kernel-only shard, 20 ticks, replay log, byte-equal `now.yaml`.
8. **Budget** — 50 eligible NPCs, one tick, ≤ 8 act, ≤ 12 intents applied.
9. **Tool scope** — tick process calling `perceive` without `as` is rejected.
10. **Proposal only** — attempt to delete a place on idle tick results in
    `state/proposals/` file, not a missing markdown file.

---

## 16. Default recommendation for v0

Ship `llm_npc_intents_per_tick: 0`.

A shard that restocks shops, drifts two relationships, hops one rumor, and
advances subsistence clocks is already “the town did not freeze.” Adding an
LLM to the idle loop before those tests pass is how FriendMos become
unrepeatable slop.

Turn the LLM on later, for **one** named NPC per tick, and only when a
kernel move would be `wait` but the NPC has an unsatisfied `goal` with
`urgency >= high`.
