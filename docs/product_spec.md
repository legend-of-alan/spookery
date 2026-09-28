# Spookery — Product spec

**Non-normative.** Engineering law for the v0.1 sitting is `architecture_v0.1.md`. `architecture_v0.md` still holds where v0.1 does not name a change. This file is business notes from early brainstorming. Where this file disagrees with either architecture file, the architecture wins.

**Working name:** Spookery (company) + `spook` (CLI / runtime) + pack (world format)
**One-line pitch:** A runtime for private worlds you describe in text: agents may intend, only the tick may write, and the world keeps moving after everyone logs off.

**Company one-liner:** Source-available kernel and pack format; paid always-on shards and conformance for authors who already write worlds with coding agents.

**Short pitch:** Friend-scale worlds as diffable packs. Models talk. The clock commits.

**Civilian hook:** Maker-shaped tools for private tick worlds that do not freeze — you write people and places in text, the clock runs subsistence when you leave. This is not “the default AI RPG.” It is the referee those worlds can share.

**Status:** Business notes. The v0 kernel and the v0.1 sitting are implemented; this file does not specify them. Idle UX for v0 stays locked here.

**Companion docs:** `docs/architecture.md` says which architecture file is law.

**Who this document is for:** Founders reading the business case. Coding agents implement the sitting from `architecture_v0.1.md`. They read `architecture_v0.md` for every rule that file does not change. This product spec does not license a code change.

**v0.5 delta:** Brand freeze. Packetry → Spookery, `packetd` → `spook` (alias `spookery`), packet → pack. Schema field `pack_rev` (was `packet_rev`). Layout file `pack.yaml` (was `packet.yaml`). Mechanics unchanged. See `BRANDING.md`.

**v0.4 delta:** Idle LLM default is 0 everywhere. Kernel intent derivation and the eligible-NPC predicate were specified. Year-0 business questions were answered then (price $9 to learn, AGPL stays, first-10-pack channel named, docks fixture authorship is days 22–45). Those engineering rules now live in `architecture_v0.md`.

**v0.3 delta:** Freeze implementation stack (Rust), pack encoding law (YAML data / Markdown flavor / JSONL logs), `now.yaml` runtime-owned, PC sheets in `state/`, hidden-string lint on bodies, official fixture uses a tiny original ruleset, and idle legibility is split: host `status` is omniscient; player resume is viewer-scoped. Kernel refusals and tick budgets did not move.

**v0.2 delta (kept):** Beachhead, paying wedge, tool/company/moonshot stack, structural ceiling, fixture-as-demo, license pick, and gates that separate “kernel shipped” from “this is a business.”

---

## 1. What we are building

An **engine and toolchain**, not a single game.

Authors describe a world as a folder of human- and model-readable text (the **pack**). A single process (the **runtime**, `spook`) loads that folder, referees play, and is the only writer of mutable world state. Friends connect through thin clients (terminal first, website later). Language models may narrate and propose actions. They never patch files or invent numbers.

The boxed loop we want (Maker-shaped):

```text
author a pack → runtime runs it → invite friends → world persists on a tick
```

The refusal that makes this a product rather than “another AI D&D”:

**A model is not allowed to write `state/`.**

The kernel is the proof of the format. The pack is how that proof travels. The spec is how we refuse a second writer. None of the three is the product alone.

- A spec without a kernel is a document.
- A kernel without a portable pack is a private World Engine (Voyage already has one).
- A pack nobody else can run is a save file.

Shells may later look like games (PWA, 2D viewport, voice). They stay skins on the same play door. They do not become the product.

---

## 2. Problem

Friend groups want a shared world that:

- does not require a human GM every session
- does not forget week-one consequences by session four
- can be customized without a game engine degree
- still exists when nobody is typed in

Incumbent AI RPG products already offer profile-driven NPCs and text worlds. They are weak on **continuity, mechanical honesty, and a format other software can run**. Reviewers of the 2026 AI RPG field scored memory and longevity as unsolved; most products are narration wearing dice.

“Someone will steal the one-sentence idea” is not the risk. The category pitch is public. The scarce thing is a **small, testable kernel** that cannot cheat — and a pack that kernel can load.

The consumer slot (“what we play tonight”) is already occupied. The open slot is a referee authors and agents can run without surrendering the table to a closed cloud.

---

## 3. Who it is for

Primary user is **not** “D&D players at large.”

**Beachhead (go-to-market order, one sentence):** authors who already live in Claude Code / Cursor and need a referee that cannot cheat.

**Who pays first:** the shard host (usually that author). Players join free. DriveThru’s default buyer is not the buyer.

| Priority | User | Job |
|---|---|---|
| 1 | Engine / tool builders, including us | Run and extend packs |
| 2 | World authors (agent-native first) | Write a folder, press Play, invite 2–8 friends, leave it ticking |
| 3 | Coding agents (Claude Code, Codex, etc.) | Edit packs and call MCP tools |
| 4 | Players in those worlds | Type what they do; do not see YAML |

Anti-users for v0: AAA MMO players, people who want a visual VTT on day one, people who want “ChatGPT with a tavern skin,” people shopping for a Friday-night AI RPG to replace Voyage.

They may play a pack later. They do not found the company.

---

## 4. Positioning

| We are | We are not |
|---|---|
| A pack format + referee + clock | An AI Dungeon / Voyage competitor |
| A kernel that makes a format real | Unreal-the-hit-studio or Jagex |
| RPG Maker–shaped tools for *living* table worlds | RPG Maker (maps + event pages) |
| Private-server runtime for small shards | A public MMO |
| Infrastructure other agents speak | A content studio balancing Woodcutting |

**Closest analogs (do not clone):**

- RPG Maker — boxed make → play → share loop; steal the shape, not event pages
- OSRS private server — tick-aligned world, data-driven shard; steal the clock, not Gielinor
- LoreKit / Grimoire MCP — agents call tools instead of hallucinating state; cousins. Delta is idle tick + knowledge firewall + conformance, not “AI NPCs”
- Craft / Voyage / Friends & Fables — creator + play platforms; they own tables; we own the format and kernel
- LLVM / Docker (ambition, not claim) — IR + runtime + tests; implementations and skins can multiply

**Differentiation claim (must stay true):**
Viewer-scoped perception, deterministic commits, replayable tick log, plot clocks that pause when the shard is empty, idle NPCs that cannot finish the module.

“AI NPCs from profiles” and “the world persists” are table stakes. Do not market them as the wedge.

---

## 5. Glossary

| Term | Meaning |
|---|---|
| **Spookery** | Company and house name. Crate / package names use `spookery`, not `spook` |
| **Pack** | A folder of text that *is* the world: canon + rules + current state. Not a network packet. `world/` remains the canon-places subtree inside a pack |
| **Runtime / `spook`** | Process that loads a pack and is the only writer of `state/`. Binary names: `spook` (preferred) and `spookery` (same file) |
| **Kernel** | Code that decides legality, dice, inventory, knowledge |
| **Tick** | One serialized step of the shard clock |
| **Intent** | Proposed move from a player, NPC agent, or kernel scheduler |
| **Effect** | Validated state mutation produced from an intent |
| **GM** | Game Master — the storyteller. In this system usually an **agent** that calls MCP, not the runtime |
| **Shard** | A running pack with durable `state/`, zero humans allowed |
| **Session** | A presence window on a shard (people are typed in) |
| **Play door** | Human client protocol (websocket / CLI) |
| **Agent door** | MCP tools |
| **Canon** | Immutable-at-runtime files under `world/`, `cast/`, `rules/` |
| **Proposal** | Queued canon change ticks are not allowed to apply themselves |
| **Wedge** | First paid offer: always-on shard + lint/conformance, host pays |

---

## 6. Product principles

1. **One writer.** Intents in. Tick or `commit_turn` writes. Nothing else.
2. **Text is the source of truth.** If humans and models cannot diff it, it does not belong in v0.
3. **Perception is scoped.** `perceive` always takes `as=player:id` or `as=npc:id`. No unscoped GM omniscience on the tick process.
4. **Numbers live in code.** Models narrate after tools return.
5. **Idle worlds are boring on purpose.** Shops restock. Plot does not self-clear.
6. **Shells are swappable.** Clients, graphics, Discord, voice — never a second authority.
7. **Tests are the spec.** If it is not in the conformance suite, it will regress.
8. **Refuse rather than flex.** Unknown moves, leaked secrets, over-budget ticks fail closed.
9. **Kernel proves the format.** A feature that cannot be expressed as a pack field plus a conformance test is not a v0 feature.
10. **Prose is not mechanics.** Markdown bodies never invent facts, flags, places, or items. If deleting the body would break a tick, the field was in the wrong file.

---

## 7. v0 scope

### In

- Pack schema (markdown / YAML)
- Runtime CLI: load pack, MCP stdio, write `state/`
- Kernel: checks, inventory/resources, knowledge firewall
- Table / tick MCP tool profiles (see §10)
- Shard tick loop, as specified in `architecture_v0.md`
- One official one-shot pack that is both a conformance fixture **and** pleasant to leave running for a week
- Terminal play loop (single human or two on one machine is enough)
- Conformance tests, as listed in `architecture_v0.md`

### Out of v0

- Public website / PWA
- Discord bot
- Spatial MMO, character controllers, netcode for movement
- Tile editor, RPG Maker event pages
- Visual VTT, battle maps
- LLM on idle ticks (`llm_npc_intents_per_tick: 0`)
- Marketplace, accounts, billing UI (price the shard on paper; do not build the store)
- Multi-ruleset soup (no Fate + COC + 5e on day one)
- Using the word “D&D” as product branding (SRD/ORC or original rules only)
- `--serve` / ugly web door in the first 45 days

### Explicit non-goals (do not “just add”)

- Replacing a human GM as the marketing headline
- Building a private RuneScape
- Unfiltered story chatbot
- Letting the model edit `cast/` or `world/` during play
- Becoming the default consumer AI RPG
- Official campaign IP as the business

---

## 8. Pack format (v0)

A pack is a directory. The layout and encoding rules are in `architecture_v0.md`.

```text
pack/
  pack.yaml                 # id, title, system, start, shard defaults
  world/
    places/<id>.md            # YAML frontmatter + optional prose
    factions/<id>.md
  cast/
    <npc>.md                  # starting NPC profile (canon)
    <pc-template>.md          # starting PC template (canon)
  rules/
    checks.yaml
    moves.yaml
    tick.yaml
  state/
    now.yaml                  # only mutable snapshot; runtime-owned
    actors/<pc-id>.yaml       # mutable PC sheets
    log/tick-NNNN.jsonl
    memories/<id>.jsonl
    rumors.yaml
    proposals/
  saves/
```

**Canon** (`world/`, `cast/`, `rules/`) is read-only at runtime.
**State** is the only live write surface.
Permanent world changes file a **proposal**; a human or studio agent applies a diff later.

**Encoding split:**

- YAML (frontmatter, rules, `now.yaml`, PC sheets) is kernel-true.
- Markdown bodies are flavor. They are never the source of facts, flags, exits, or stock.
- JSONL is the audit tape. It is not a second `Now`.

`now.yaml` is not hand-edited mid-shard. A hand edit is a migration that must parse and re-commit. After a crash, `state_rev` on disk wins.

Player characters: starting template lives in `cast/`. The live sheet lives in `state/actors/<pc-id>.yaml`. Players do not write `cast/` during play.

NPC profile (conceptual fields, not final schema):

- identity, voice, goals, needs
- `hidden` vs public facts (fact ids)
- relationships / stance targets
- `tick` block: eligible, range, schedule, stock_rate
- allowed flag keys

Hidden-string lint fails closed: a Markdown body may not contain another entity’s hidden fact text.

Rulesets declare legal `move` enums. Unknown moves reject.

`rules/tick.yaml` defaults live in the tick spec. v0 default: kernel-only idle ticks.

### 8.1 Official fixture ruleset

The bundled one-shot (`packs/oneshot-docks/`) ships a **tiny original** ruleset, not an SRD or ORC subset.

Why: v0 is proving a referee and a pack, not shipping a well-known game. The spec already forbids branding as D&D. An SRD subset pulls in license text, skill lists, and combat gravity the docks fixture does not need. A second implementer should add a move by editing `moves.yaml` + a test, not by reading 5e.

Minimum idle moves (must exist in the fixture `rules/moves.yaml`):

```text
wait travel work restock rest pay_upkeep
drift_relationship spread_rumor leave_note file_proposal advance_clock
```

Live-only moves the fixture may also declare (rejected when `presence.humans == 0`):

```text
talk attempt attack give take
```

Checks stay tiny: one `resolve_check` profile (e.g. 2d6 + stat vs a target number) is enough. Do not import ability-score blocks, spell lists, or XP.

An SRD/ORC **pack** may exist later as a different `rules/` folder. v0 does not load two rulesets at once. Genre packs choose their own pause-policy defaults; the docks fixture uses the tick-spec defaults in §8.2.

### 8.2 Docks fixture pause policy

```yaml
# packs/oneshot-docks/rules/tick.yaml
pause_policy:
  named_plot_clocks: pause_when_empty
  subsistence_clocks: always_run
  relationship_drift: always_run
  travel: run_if_schedule
llm_npc_intents_per_tick: 0
```

That is the tick spec default. The fixture must be authored so those four lines produce a week a *host* can confirm in `spook status` and a *player* can discover in place via `perceive` (see §11). Off-screen stock is not announced on login.

---

## 9. Runtime

`spook` is one OS process.

```text
  PLAYERS                      AUTHORS / AGENTS
  client (later: website)      CLI / Claude Code / Cursor
        | play door                    | agent door (MCP)
        v                              v
  +------------------------------------------+
  |                 spook                  |
  |   kernel    tick loop    tool handlers   |
  +-----------+------------------+-----------+
              |                  |
        state/now.yaml      pack canon
        tick log
```

**Play door:** human intents (“I search the desk”).
**Agent door:** MCP.
Same permissions model: players cannot `apply_effect`; tick process cannot `perceive` as GM; studio tools are a separate profile.

Deployment:

| Mode | Command-level idea | When |
|---|---|---|
| Laptop | `spook ./pack` | Thursday / tests / first 45 days |
| Shard host | `spook ./pack --tick-every 300 --serve` | After kernel + fixture exist |
| Hosted studio | same binary, multi-tenant later | After other authors exist |

The runtime is not the LLM. Unplug API keys and subsistence ticks still run.

Graphics are not in this process. A future 2D client reads state and submits intents. If a client starts resolving hits locally, it is a fork, not an extension.

---

## 10. MCP surface

Two servers or two tool profiles on one process. Do not dump CRUD.

### 10.1 Table / live (`pack-table`)

`session_open`, `session_status`, `perceive`, `intent`, `resolve_check`, `apply_effect`, `npc_act`, `commit_turn`, `recall`, `session_save`

### 10.2 Tick (`pack-tick`)

Allowlist only: `session_status`, `perceive` (must pass `as=npc:id` or system actor), `recall`, `npc_act` (budgeted; off by default), `resolve_check`, `apply_effect`, `tick_commit`

Denylist for tick process: `patch_entity`, unscoped `perceive`, studio tools, inventing effect classes.

### 10.3 Studio (`pack-studio`)

`pack_info`, `read_entity`, `patch_entity`, `lint_pack`, `simulate_knowledge`, `run_conformance`, `compile_preview`

Tool design rules:

- Outcome-sized names, not REST
- Descriptions written for models (when to use, side effects)
- Stable `tools/list` order
- Writes return diffs
- Destructive ops audited
- Target ~10–17 tools per profile; do not grow to 50 without traces showing missed calls

Full tick tool contracts are in `architecture_v0.md`.

---

## 11. Play loop

1. Client sends player text as `intent`.
2. Runtime queues; enforces turn / presence.
3. GM agent (optional) calls `perceive` for relevant actors.
4. Kernel resolves checks and effects.
5. `commit_turn` or `tick_commit` atomically updates `state/` + log.
6. Clients receive public narration and viewer-scoped private notes.

Players never see MCP, keys, or YAML in the intended UX. v0 may be terminal-only and still count.

Idle / overnight is specified in `architecture_v0.md`. Summary: subsistence clocks run; named plot clocks pause when no humans are present; max 12 catch-up ticks; relationship drift ±1/day; rumors hop by fact id only.

Idle life must be **legible**, not only buried in `now.yaml`. Legible does **not** mean a player wakes up omniscient.

Split the digest by viewer:

| Surface | Who | What it may show |
|---|---|---|
| `spook status` (host / studio / tests) | Shard host | Full structured idle summary: stock deltas, notes, rumor hops, paused clocks. This proves the fixture. It is not player-facing. |
| Session resume / `perceive(as=player:id)` | That player | Only events the viewer is allowed to know |

**Player-visible on return** (no extra looking):

- A `message` targeted at them, or left at their last place / claimed home
- A rumor that hopped onto *their* `known_by` (or onto someone in their current place who would tell them — still a perceive at that place)
- A named plot clock they already own or started (`requires_presence` stayed paused)

**Player-visible only in place:**

- Shop stock, hours, door notes on the shop, who is behind the counter. These appear when the player is at that place and `perceive`s, or asks. They do not appear as a login banner.

A returning player should not be told `docks_shop lamp_oil 12 → 16` while they are still in their room. That is unscoped perception wearing a digest.

The docks fixture is authored so both layers exist after a week idle:

1. Host `status` shows all four effect classes fired (stock, note, rumor, paused clock).
2. A player who walks back to the shop can see the restock and the door note through ordinary `perceive`.
3. That player’s resume is not a newspaper of the whole harbor.

No model writes the digest. It filters JSONL effect classes by `as=`. It does not invent sentences the log does not support. If the only way for a *host* to know the world moved is to diff `now.yaml`, criterion 9 fails. If a *player* learns off-screen stock without going there, the firewall failed.

---

## 12. Client roadmap (shells, not products)

| Stage | Client | Notes |
|---|---|---|
| v0 | CLI | Required. First 45 days: this only |
| v0.5 | Minimal web log + input on `--serve` | After fixture survives a week idle |
| v1 | PWA | Same play door |
| optional | Discord bot | Same play door |
| later | Tick-aligned 2D viewport | Click/tap → intent; interpolates only |
| later | Voice | Speech-to-intent, TTS out |

Website is a skin. Do not start there. A prettier client does not make this the default AI RPG.

---

## 13. Acceptance criteria

### 13.1 Kernel shipped (v0 engineering)

A reviewer or agent should treat the kernel as shipped only if all of these pass:

1. Load a pack from a folder and speak MCP on stdio.
2. `perceive(as=npc:mira)` never contains Mira’s `hidden` strings.
3. After 100 idle ticks, Aldric still does not know Mira’s hidden fact (tick spec test 1).
4. A plot clock tagged `requires_presence: true` does not advance across 12 empty-shard ticks.
5. Shop stock changes only within `stock_rate` and never goes negative.
6. Killing the process between `apply_effect` and `tick_commit` leaves the previous `state_rev`.
7. Kernel-only 20-tick replay produces byte-equal `now.yaml`.
8. 50 eligible NPCs, one tick: ≤ 8 act, ≤ 12 intents applied.
9. One bundled one-shot pack plays in the CLI without editing code **and** is pleasant to leave running for a week. Host `status` shows stock / note / rumor / paused clock fired. A player resume does not leak off-screen stock; walking to the shop and `perceive`ing is enough to see the restock and door note.
10. README + this spec + tick spec + encoding law are enough for another agent to add a move to `moves.yaml` and a test.
11. Strip-bodies: delete every Markdown body in `packs/oneshot-docks`; tests 1, 2, 3, 5, 6, and 7 still pass. Tests 4 and 8 stay on the packs those tests build.
12. Hidden-string lint: fixture bodies contain none of Mira’s hidden fact text.

### 13.2 Company gates (not the same checkbox)

v0 engineering can pass while the business is still a gist. Treat these as separate:

| Gate | Target | Meaning |
|---|---|---|
| External packs | 10 worlds we did not write | Format is usable |
| Lived-in shards | 3 shards survive 7 real days with ≥2 humans returning | Idle contract is a toy people keep |
| Paid renewal | ≥1 stranger renews a shard | Wedge is not theoretical |
| Cost | Idle shard-day cost measured and boringly low | Kernel-only path is real |
| Kill | People say “cool demo” and paste the prompt into ChatGPT | Stop |

Do not raise, brand, or marketplace before the first four trend the right way.

---

## 14. Business case

### 14.1 Three outcomes (do not collapse them)

| Outcome | What people use | Analog |
|---|---|---|
| **Tool** | `spook` runs packs | LoreKit, a good Foundry module |
| **Company** | Runtime + paid shards/studio | RPG Maker, Foundry, early Unity |
| **Moonshot** | Worlds *are* packs; other clients speak the contract | Unreal-if-authors-stay, or git-if-others-implement |

The kernel is the moonshot *engine*. The format is the moonshot *surface*. The spec is the discipline that keeps them one project. Hosting and conformance are where a company bills.

Do not pitch “format other software can run” as the year-0 offer. That is year-3 language. Year-0 offer is a shard that stays honest.

### 14.2 Why this can exist

- Groups still cannot schedule human GMs reliably.
- Hosted AI tables already charge ~$20–40/month with friends-play-free — willingness to pay for a *table* is proven. Willingness to pay for a *format* is not. Do not confuse them.
- The crowded layer is *chat*. The thin layer is *a runnable, honest referee*.
- Coding agents need schemas and tools, not another GUI. MCP + pack is the 2026 distribution channel for engine work.
- Memory and longevity are the unsolved axes in the 2026 field. The refusals aim there on purpose.

### 14.3 Why “AI NPCs from profiles” is not the business

That is table stakes (character cards, Voyage NPCs, etc.). The business is the **protocol plus the kernel that enforces it**: pack + firewall + tick + tests other runtimes can implement.

### 14.4 Paying wedge (one sentence)

Engine source-available; **always-on shard + lint/conformance paid**; host pays, friends free; BYOK for the live GM agent; kernel idle included.

Starting price to learn from, not a freeze: **$9 per shard-month** (band $8–15). Price like always-on hosting plus lint, not like Voyage/Fables inference tables. Measure cost per idle shard-day before treating $9 as real. Studio seats come after shards, not before. Full answers: `BUSINESS_CASE.md`.

### 14.5 Who pays (in order of alignment)

1. **Hosted playtest shards + conformance** — engine OSS or source-available; workspace, lint, always-on shard are paid (RPGJS / open-core pattern).
2. **Studio seats** — authors who already have packs.
3. **B2B kernel** — platforms that do not want to rewrite isolation + commit. Assume Voyage/Craft will not be this buyer.
4. **Pack registry** — tax verification (“this pack passed isolation on model X”), not creativity. Year-3. Nobody pays this until untrusted packs have burned someone.

Do not take 30% of someone else’s game as v1. That is a platform company.

### 14.6 Unit economics constraint

A 4-player turn that spins a frontier model per NPC is a money fire. v0 idle path is kernel-only. Live path: one GM agent, small models for banter, kernel for numbers. Target to learn later: well under $1 per player-hour on mid models.

**Ship gate:** if the idle path depends on a frontier model, the company is dead. This is not a cost note. It is a kill switch.

### 14.7 Structural ceiling

Better tools and clients can make this a great way to **play a pack**. They cannot, by themselves, make Spookery the default consumer AI RPG.

A default consumer AI RPG wins on zero authoring, a library of worlds, instant multiplayer, narration that always has something to say, and a world that feels busy while you are away. The refusals cut against all five: content is authored, plot clocks pause when empty, shards are friend-scale, the interesting object is a folder plus a kernel.

That is not a missing UI. That is the product.

“Living worlds of all kinds” is in-bounds when it means: discrete-tick, authored, friend-scale worlds in any genre pack, with later 2D or voice shells on the same state.

Out of bounds without a second writer: public MMO, physics-as-truth, model-dreamed canon on the idle clock, “the city lived a novel while you slept.”

### 14.8 Moonshot clause

Moonshot is **not** “Spookery becomes what people mean by AI RPG.”

Moonshot is: **the pack becomes the durable object humans and agents check out, run, and leave ticking** — the git of private worlds. Clients become commodities. We tax hosting, verification, and studio.

Two shapes, same spec:

- **Unreal-shaped (we can choose):** `spook` is the best place to run a pack; studio tools make leaving expensive.
- **Git-shaped (tail):** other kernels implement the same pack because the tests demand it.

Unreal-the-company also needed a hit studio. That blink is **path B** below. Forbidden as *this* project.

Path A vs path B, written down so a roadmap fight has to say the name:

| | A — this project | B — a different firm |
|---|---|---|
| Ambition | Default referee other living-world games run on | Default place people play AI RPGs |
| Requires | Kernel + beloved pack + other clients | Official worlds, owned client, marketplace, growth |
| Allowed? | Yes | No. Second writer. New project. |

Annual question remains: *shell, pack, or second writer?* Second writer means a new project.

### 14.9 IP and community

- Official fixture ships original rules (§8.1). Do not brand as D&D. SRD/ORC packs are later, optional, and separate.
- Core TTRPG hobby is AI-skeptical. Beachhead is AI-native authors and small friend groups, not DriveThru’s default buyer.
- Some TTRPG storefronts ban AI content; do not plan that channel.

### 14.10 Good-idea test

Rebuild a category when you **refuse** something incumbents allow. We refuse model-written state, idle plot completion, and unscoped perception. If a roadmap item does not serve those refusals, it is Premiere-but-ours.

---

## 15. 90-day build plan

Assumes a laptop is available. Phone week is notes only.

**Days 1–21**
Pin YAML crate. Freeze the pack schema against `architecture_v0.md`. Implement knowledge firewall + atomic `state/` writeback. Port tests 1, 2, and 6 as empty fixtures. No UI. No `--serve`.

**Days 22–45**
MCP stdio: `perceive`, `apply_effect`, `tick_commit`. CLI play. Bundled one-shot that is fun to idle, not only a secret-leak test — authorship pass required (3 named NPCs, shop, note, rumor, paused plot clock). BYOK unused if kernel-only. Still no website.

**Days 46–70**
Tick scheduler. `rules/tick.yaml`. Tests 3–5, 7–8. JSONL audit log. Measure cost per idle shard-day on a laptop process.

**Days 71–90**
Ask a written list of 20 agent-native authors (see `BUSINESS_CASE.md` §4). Optional ugly `--serve` only after the fixture has survived a week idle. Success = someone runs a world they wrote against this runtime without rewriting the schema **and without you on the call**.

Kill signal: people say “cool demo” and paste the prompt into ChatGPT instead.

Do not raise, brand, or marketplace in this window.

---

## 16. 10-year extension path

The core does not grow a second writer. Extensions are shells and packs.

| Horizon | Add |
|---|---|
| 0–1y | Kernel + one beloved pack + first paid shards |
| 1–3y | Maker loop: invite link, other authors, lint/studio, ugly web door |
| 3–5y | Optional tick-aligned 2D client (click → intent) |
| 5–7y | Hosting at more than hobby scale, registry, versioned rules packs |
| 7–10y | Format other clients speak — *or* we blink and become a content studio |

**Not inherently text-only.** Text is the honest first client and the debug view. An OSRS-like viewport is valid later if position, skills, and inventory are state fields and the client only interpolates.

**Inherently limited to:** discrete ticks, budgeted agents, small shards (about 2–16 humans), content supplied by packs/authors.

**Private OSRS analog (allowed reading):** tick machine + data-driven shard + friends host it.
**Private OSRS analog (forbidden reading):** we become Jagex.

---

## 17. Repo layout (suggested)

```text
/docs
  architecture_v0.1.md        # frozen normative spec for the v0.1 sitting
  architecture_v0.md          # frozen v0 spec; holds where v0.1 does not name a change
  architecture.md             # index of the two files above
  product_spec.md          # this file; business notes, not law
/schema                    # now.schema.json, effect.schema.json, intent.schema.json
/rulesets                  # default original moves, checks, tick.yaml
/packs
  oneshot-docks/           # official fixture + week-idle demo
/src
  kernel/
  mcp/
  tick/
  cli/
/tests
  conformance/
```

Language: Rust, as pinned in `architecture_v0.md`.

Agents implement; a human reviews. Agents implementing features MUST add or update a conformance test in the same change.

---

## 18. License and remaining open questions

### Decided

- **Runtime license:** AGPL-3.0-or-later (network use must share). Alternative if AGPL blocks a specific B2B conversation: source-available with an explicit Commons Clause / “cannot offer competing hosted shards” rider — record the change; do not silently flip.
- **Spec + schema + official fixture text:** CC BY 4.0, so a second implementer can speak the pack.
- **Brand:** Spookery / `spook` / pack. Still do not brand as D&D. Details in `BRANDING.md`.
- **Runtime language:** Rust. See `STACK.md`.
- **Entity files:** `.md` with YAML frontmatter. Bodies are non-authoritative.
- **`now.yaml`:** runtime-owned. No mid-shard hand edits.
- **PCs:** template in `cast/`, live sheet in `state/actors/`.
- **Fixture rules:** tiny original list in §8.1. Not SRD/ORC.
- **Docks pause policy:** tick-spec defaults in §8.2.
- **Hidden strings:** lint fails if they appear in any Markdown body.

### Not blockers

- Exact YAML crate (pick one in week 1; pin it; golden-file the emitter).
- Exact YAML crate versions (not the crate itself).
- Pause-policy overrides in *later* genre packs.

Closed in v0.4 / `BUSINESS_CASE.md`: shard list price to learn ($9), AGPL stays, first-10-pack channel, docks authorship in days 22–45, no raise this window.

Do not block v0 on the remaining items. Pick, record, move.

---

## 19. Review checklist for humans / agents

When reviewing this spec, answer:

1. Is the refusal (“models do not write state”) still load-bearing in every section?
2. Did any paragraph smuggle in a website, MMO, or event-page editor as v0 work?
3. Can a second implementer build the kernel from this + the tick spec + tests?
4. Is the business still “format + runtime + studio,” not “our official campaign”?
5. Is the paying wedge still “shard + conformance,” or did a paragraph smuggle marketplace / official campaign / default-AI-RPG?
6. Is the bundled pack a world someone would leave running for a week, or only a secret-leak test double?
7. Is the kernel still the proof of the format (tests), not a closed World Engine and not a spec-only hope?
8. What is the smallest change you would make before implementation?

Suggested first implementation task after review: **conformance test 1** (hidden fact never appears in another actor’s `perceive` or memories), then the `perceive` tool that makes it pass.
