# Now + effect payloads — v0

**Status:** Frozen for v0 implementation.  
**Companions:** `BRANDING.md`, `PRODUCT_SPEC_v0.5.md`, `SHARD_TICK_SPEC_v0.3.md`, `PACK_ENCODING_v0.3.md`, `STACK_v0.3.md`.

This is the data contract the kernel types as `Now` and as `Effect`.  
JSON Schema in `/schema` must match these shapes. Prose in other docs yields to this page.

Machine-readable copies to land in the repo on commit 1:

```text
/schema/now.schema.json
/schema/effect.schema.json
/schema/intent.schema.json
```

Until those files exist, this page is the schema.

---

## 1. `state/now.yaml`

Typed Rust struct `Now`. Disk YAML is a projection of that struct from a pinned emitter.  
Byte-equal `now.yaml` after kernel-only replay is the ship gate.

```yaml
schema: now/v0
shard_id: docks-001
pack_rev: "blake2s:…"          # sorted canon bytes; refuse ticks on mismatch
state_rev: 9002                  # +1 per successful tick_commit
tick_index: 1842                 # +1 per tick loop that runs
world_time: 10320                # minutes since pack epoch
last_tick_at: "2026-09-21T18:00:00Z"

presence:
  humans: 0
  session_ids: []
  actors_present: []             # actor ids typed in

pause:
  named_plot_clocks: paused      # paused | running
  subsistence_clocks: running
  relationship_drift: running
  travel: running

places:
  docks_shop:
    stock: { lamp_oil: 16 }
    hours: open                  # open | closed
    notes: [note_mira_door]      # ids; bodies live in messages[]
    actors: [mira]

actors:
  mira:
    kind: npc
    place: docks_shop
    flags: []
    busy: false
    needs: { coin: 12, rest: 2 }
    resources: { coin: 12 }
    relationships:
      aldric: { stance: 0, target: -1, last_interaction_world_time: 10080 }
    known_facts: [fact_ledger_short]
    tick:
      last_act_tick: 1840
      idle_acts_today: 1
      live_acts_this_hour: 0

  pc_rowan:
    kind: pc
    place: mira_rooms
    sheet: state/actors/pc_rowan.yaml
    flags: []
    busy: false
    known_facts: []

clocks:
  assassination_debt:
    value: 3
    max: 8
    requires_presence: true
    owners: [pc_rowan]
    paused: true
  shop_day:
    value: 4
    max: 7
    requires_presence: false
    owners: [mira]
    paused: false

rumors:
  - id: r_184
    fact_ids: [fact_ledger_short]
    origin: mira
    heat: 1
    known_by: [mira, aldric]

messages:
  - id: note_mira_door
    to: [pc_rowan]
    place: docks_shop
    fact_ids: []
    text: "Back at dusk. Oil on the shelf."

proposals: []                    # ids of files in state/proposals/
```

Rules:

- Unknown top-level keys fail load.
- `actors.<pc-id>` is a pointer plus live flags/place/facts. The mutable sheet stays in `state/actors/<pc-id>.yaml`.
- NPC live fields live only here. Canon voice/goals/hidden/schedule stay in `cast/<id>.md` frontmatter.
- After a crash, `state_rev` on disk wins. A JSONL line without a matching `state_rev_after` is discarded.

---

## 2. Intent (input to the tick)

```yaml
id: "uuid"
tick_index: 1842
actor_id: mira
actor_kind: npc          # npc | player | system
source: kernel           # kernel | llm | player | system
move: restock            # enum from rules/moves.yaml
target:
  place: docks_shop      # optional; at most one of place | actor | clock | rumor
  actor: null
  clock: null
  rumor: null
args:                    # move-specific; see §4
  sku: lamp_oil
  delta: 4
rationale_code: schedule.dusk_close
llm_say: null            # never trusted
```

Unknown `move` → reject. Live-only moves (`talk attempt attack give take`) reject when `presence.humans == 0`.

---

## 3. Effect (output of `apply_effect`)

Every applied intent expands to ≤ `max_effects_per_intent` (default 4) effects.  
Only these `class` values exist in v0. Unknown class → `ILLEGAL_CLASS`.

Common envelope:

```yaml
class: stock
as: "npc:mira"           # viewer who caused it; required
intent_id: "uuid"
payload: {}              # class-specific, required
```

### Payloads

**`clock`**
```yaml
clock_id: shop_day
delta: 1                 # integer; may be 0
```
Fails `PAUSED_CLOCK` if `requires_presence` and no humans. Fails `UNKNOWN_KEY` if clock is not in `now.clocks`.

**`need`**
```yaml
actor_id: mira
key: rest                # must be in that actor's profile needs
delta: -1
```
Clamp to profile range. Cannot invent keys.

**`stock`**
```yaml
place: docks_shop
sku: lamp_oil
delta: 4                 # signed; result must be ≥ 0
```
`|delta|` must be ≤ declared `stock_rate[sku]` for this actor on this tick. Fails `OUT_OF_RANGE` otherwise. Fails `NOT_OWNER` if the actor's `range` / schedule does not include the place.

**`location`**
```yaml
actor_id: mira
from: docks_shop
to: mira_rooms
```
`to` must be in profile `range` and on the place graph / schedule. No teleport.

**`relationship`**
```yaml
a: mira
b: aldric
delta: -1                # -1, 0, or +1
```
At most one apply per pair per world-day. Skip if either `busy` or `last_interaction_world_time` within 1 world-day.

**`reputation`**
```yaml
actor_id: mira
faction: docks
delta: -1                # -1, 0, or +1
```
At most one per faction per world-day.

**`memory`**
```yaml
actor_id: mira
fact_ids: [fact_sold_four_oil]
tags: [work, uneventful]
text: "Closed shop. Sold four lamp oil."   # optional, ≤ 240 chars
```
`text` and `fact_ids` may only contain facts in that actor's `known_facts` after the tick's prior effects. Conformance greps secret strings.

**`rumor`**
```yaml
rumor_id: r_184          # existing or new
fact_ids: [fact_ledger_short]
from: mira
to: aldric
heat: 1
```
Recipient must be allowed the fact under the firewall. Else fail closed (`ILLEGAL_KNOWLEDGE`). Hop counts against `max_rumor_hops`.

**`resource`**
```yaml
actor_id: mira
key: coin
delta: -2
```
Cannot mint a key not listed on the profile or the live sheet.

**`flag`**
```yaml
actor_id: mira
key: busy
value: false             # set or clear only keys listed on the profile
```

**`proposal`**
```yaml
proposal_id: p_burn_shed
kind: delete_place       # delete_place | patch_entity | other
target: world/places/shed.md
summary: "Shed burned."
```
Writes `state/proposals/<id>.yaml`. Never edits `world/` or `cast/`.

**`message`**
```yaml
message_id: note_mira_door
to: [pc_rowan]           # actor ids; may be empty if place-only
place: docks_shop        # optional
fact_ids: []
text: "Back at dusk. Oil on the shelf."
```
Queued. Not scene prose. Not a login banner unless `to` includes the returning player.

---

## 4. Move → effect map (idle enum)

Kernel expands a legal move using this table. Do not invent a fifth effect from flavor text.

| Move | Effects |
|---|---|
| `wait` | none (does not count against `max_npcs_acting`) |
| `travel` | `location` |
| `work` | `need` and/or `resource` as the profile `work` block declares |
| `restock` | `stock` |
| `rest` | `need` (rest) |
| `pay_upkeep` | `resource` and/or `need` |
| `drift_relationship` | `relationship` |
| `spread_rumor` | `rumor` |
| `leave_note` | `message` |
| `file_proposal` | `proposal` |
| `advance_clock` | `clock` |

Live-only moves (`talk attempt attack give take`) are not expanded on idle ticks.

---

## 5. PC sheet `state/actors/<pc-id>.yaml`

```yaml
id: pc_rowan
template: cast/pc-harbormouse.md
stats: { grit: 1, wit: 1, nerve: 0 }
resources: { coin: 6 }
inventory: [lamp]
flags: []
```

Players never write `cast/` during play. A sheet field that should become canon is a `proposal`.
