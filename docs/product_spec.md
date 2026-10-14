# Spookery — Product spec

**Non-normative.** Business notes. Engineering law is `docs/spec.md`. Where this file disagrees with that spec, the spec wins. This file does not fill a rule the spec does not state.

**Names:** Spookery (company), `spook` (CLI / runtime), pack (world format). The crate is `spookery`.
**One-line pitch:** A runtime for private worlds you describe in text: agents may intend, only the tick may write, and the world keeps moving after everyone logs off.

**Company one-liner:** Open source (AGPL-3.0) kernel and CC BY 4.0 pack format; paid always-on shards and conformance for authors who already write worlds with coding agents.

**Short pitch:** Friend-scale worlds as diffable packs. Models talk. The clock commits.

**Civilian hook:** Maker-shaped tools for private tick worlds that do not freeze — you write people and places in text, the clock runs subsistence when you leave. This is not “the default AI RPG.” It is the referee those worlds can share.

**Status:** Business notes. This file does not specify the kernel or the sitting.

**Who this document is for:** Founders reading the business case. This product spec does not license a code change.

---

## Vision (Oct 8, 2026)

No releases yet. No compatibility guarantees.

- A generic RPG referee engine. Authors write a pack and host it like a private server.
- One GM seat (AI or human) and 1 to 3 player seats. Each player sits a distinct pack PC. A 4th player gets `SESSION_FULL`. A hello for a PC another seat holds gets `SEAT_TAKEN`. That seat model is built.
- Players walk around and take basic actions.
- Packs are written and updated by people and by coding agents (Grok, Claude Code, Cursor), so the format must be machine-checkable: every refusal has a file, a location, a stable code and a fix hint (`spook check`, `spook check --json`, and generated schemas later). `spook check` is not built yet (#24). `spook check --json` is not built yet (#26). Generated schemas are not built yet (#29).
- The rules primitives must be able to express a 5e-style game with no 5e-specific code. Generic engine, not a 5e replacement, and no SRD text. `packs/sample-d20` is that sample. Known gaps: a difficulty taken from the target's defense meter, rolled effect amounts, advantage-style rolls, a modifier from equipment, a condition when hit points reach zero, and consume an item (drink checks for the tonic but never uses it up, so it heals forever).

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
| 2 | World authors (agent-native first) | Write a folder, press Play, sit one GM (you or an AI) and up to three players, leave it ticking |
| 3 | Coding agents (Claude Code, Codex, etc.) | Edit packs and call MCP tools. Use `http://127.0.0.1:9738/mcp` from `spook run`. `--mcp-http` overrides that port. One GM seat, for playtesting and authoring loops; `spook ai-gm` is the shipped model GM |
| 4 | Players in those worlds | Type what they do; do not see YAML |

Packs are written and updated by people and by coding agents (Grok, Claude Code, Cursor). The format must be machine-checkable: every refusal has a file, a location, a stable code and a fix hint (`spook check` is not built yet, #24; `spook check --json` is not built yet, #26; generated schemas are not built yet, #29).

Anti-users: AAA MMO players, people who want a visual VTT on day one, people who want “ChatGPT with a tavern skin,” people shopping for a Friday-night AI RPG to replace Voyage.

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
- LoreKit / Grimoire MCP — agents call tools instead of hallucinating state; cousins. The difference is idle tick + knowledge firewall + conformance, not “AI NPCs”
- Craft / Voyage / Friends & Fables — creator + play platforms; they own tables; we own the format and kernel
- LLVM / Docker (ambition, not claim) — IR + runtime + tests; implementations and skins can multiply

**Differentiation claim (must stay true):**
Viewer-scoped perception, deterministic commits, replayable tick log, plot clocks that pause when the shard is empty, idle NPCs that cannot finish the module.

“AI NPCs from profiles” and “the world persists” are table stakes. Do not market them as the wedge.

---

## 5. Glossary

| Term | Meaning |
|---|---|
| **Spookery** | Company and house name. The library is `spookery`. The package and the binary are `spook` |
| **Pack** | A folder of text that *is* the world: canon + rules + current state. `world/` is the canon-places subtree inside a pack |
| **Runtime / `spook`** | Process that loads a pack and is the only writer of `state/`. The binary is `spook` |
| **Kernel** | Code that decides legality, checks, and knowledge |
| **Tick** | One serialized step of the shard clock |
| **Intent** | Proposed move from a player, NPC agent, or kernel scheduler |
| **Effect** | Validated state mutation produced from an intent |
| **GM** | Game Master — the storyteller. In this system usually an **agent** that calls MCP, not the runtime. `spook ai-gm` is the shipped model GM; a coding agent on the one GM seat playtests and authors |
| **World** | A running pack with durable `state/`, zero humans allowed |
| **Session** | A presence window on a shard (people are typed in) |
| **Play door** | Human client on the CLI, over loopback |
| **Agent door** | MCP tools at `http://127.0.0.1:9738/mcp` from `spook run`. `--mcp-http` overrides that port |
| **Canon** | Immutable-at-runtime files under `world/`, `cast/`, `rules/` |
| **Wedge** | First paid offer: always-on shard + lint/conformance, host pays |

---

## 6. Product principles

1. **Text is the source of truth.** If humans and models cannot diff it, it does not belong in the product.
2. **Numbers live in code.** Models narrate after tools return.
3. **Idle worlds are boring on purpose.** The shop keeps living. The plot does not finish itself while the table is away.
4. **Shells are swappable.** Clients, graphics, Discord, voice. None of them is a second product.
5. **Kernel proves the format.** A feature that cannot be expressed as a pack field is not a feature.

---

## 7. Scope

### In

- Pack schema (markdown / YAML)
- Runtime CLI: load a pack and write `state/`
- Kernel: checks, meters, habits, knowledge firewall
- World tick loop
- One official one-shot pack that is both a conformance fixture **and** pleasant to leave running for a week
- Terminal play: one GM seat and up to three player seats. A fourth player gets `SESSION_FULL`. A held character gets `SEAT_TAKEN`. Built.
- Rules primitives able to express a 5e-style game in a pack, with no 5e-specific code and no SRD text. `packs/sample-d20` is that sample. Known gaps: a difficulty taken from the target's defense meter, rolled effect amounts, advantage-style rolls, a modifier from equipment, a condition when hit points reach zero, and consume an item (drink checks for the tonic but never uses it up, so it heals forever).
- Conformance tests

### Out of scope

- Public website / PWA
- Discord bot
- Spatial MMO, character controllers, netcode for movement
- Tile editor, RPG Maker event pages
- Visual VTT, battle maps
- LLM on idle ticks
- Marketplace, accounts, billing UI (price the shard on paper; do not build the store)
- Bundled multi-system content (Fate, COC, and 5e together) and SRD text
- Using the word “D&D” as product branding (SRD/ORC or original rules only)
- A web door, or any bind that is not loopback

### Explicit non-goals (do not “just add”)

- Replacing a human GM as the marketing headline
- Building a private RuneScape
- Unfiltered story chatbot
- Letting the model edit `cast/` or `world/` during play
- Becoming the default consumer AI RPG
- Official campaign IP as the business

---

## 8. Pack format

A pack is a directory.

```text
pack/
  pack.yaml                 # id, title, system, start, shard defaults
  world/
    places/<id>.md            # YAML frontmatter + optional prose
  cast/
    <npc>.md                  # kind npc (canon)
    <pc>.md                   # kind pc_template (canon)
  rules/
    checks.yaml
    meters.yaml
    verbs.yaml
    tick.yaml
  state/
    save.yaml                  # live world; each PC is an actor here
    journal/session.jsonl     # speech log, runtime-created
    journal/studio.jsonl      # migrate record, runtime-created
    journal/tick-NNNN.jsonl   # one file per committed tick, runtime-created
    memories/<id>.jsonl       # runtime-created
```

The runtime creates `state/.tick.lock` and `state/.session.lock`.

**Canon** (`world/`, `cast/`, `rules/`) is the authored text.
**State** is what the runtime keeps while the world runs.
A permanent change to the authored world waits for a person. Players do not rewrite `cast/` during play.

Format tags carry no version until the format is frozen and a compatibility promise is made. The tags are `pack`, `place`, `cast`, `meters`, `moves`, `checks`, `tick`, and `now`.

Authors write YAML and Markdown. Markdown bodies are flavor for people reading the pack. A PC is a `pc_template` cast file plus a live actor in `state/save.yaml`.

NPC profile, as a product sketch:

- identity, voice, goals, meters, habits
- what the table is allowed to know, and what it is not
- relationships (parked in #36)
- where an NPC works, when, and what the shop does

Idle ticks are kernel-only. The world keeps moving after everyone logs off. The plot does not finish itself.

### 8.1 Official fixture ruleset

The bundled one-shot (`packs/oneshot-docks/`) ships a **tiny original** ruleset, not an SRD or ORC subset.

Why: the work is proving a referee and a pack, not shipping a well-known game. The product does not brand as D&D. An SRD subset pulls in license text, skill lists, and combat gravity the docks fixture does not need. A second author adds a move in the pack. The rules primitives must express a 5e-style game, with no 5e-specific code and no SRD text (#25).

The docks fixture does not import ability-score blocks, spell lists, or XP.

An SRD/ORC **pack** may exist later as a different `rules/` folder. The runtime does not load two rulesets at once.

### 8.2 Docks fixture, a week later

The fixture is a world a host can leave running and a player can return to. The host can tell the week happened. The player finds the shop by being there. Off-screen stock is not announced on login.

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
        state/save.yaml      pack canon
        tick log
```

**Play door:** human intents (“I search the desk”).
**Agent door:** MCP. Use `http://127.0.0.1:9738/mcp` from `spook run`. `--mcp-http` overrides that port. One GM seat, for playtesting and authoring loops; `spook ai-gm` is the shipped model GM.

Deployment:

| Mode | Shape | When |
|---|---|---|
| Laptop | `spook run` on loopback | The sitting |
| World host | The same binary, left running | When a host wants the pack always on |
| Hosted studio | Multi-tenant, later | After other authors exist |

The runtime is not the LLM. Unplug API keys and subsistence ticks still run.

Graphics are not in this process. A future 2D client reads state and submits intents. If a client starts resolving hits locally, it is a fork, not an extension.

---

## 10. The menu

The seat's `tools/list` is the menu. Tool names stay outcome-sized. The list does not grow because a model asked for REST. Descriptions say when to use a tool and what it changes. Order stays stable.

---

## 11. Play loop

1. A player types a line.
2. The runtime holds it until the table commits a turn, or the clock runs.
3. A GM, if one is seated, can look at the table.
4. The clock resolves what the line and the world do.
5. The world and its log update together.
6. Players hear what their character can know.

Players never see MCP, keys, or YAML in the intended UX. A terminal-only build still counts.

Overnight, shops keep living and the plot waits. Idle life must be **legible**. Legible does not mean a player wakes up omniscient.

A host can tell the week happened. A player finds the shop by walking there. A returning player is not handed a newspaper of the whole harbor, and is not told the shop's stock while they are still in their room.

No model writes that summary. If the only way for a host to know the world moved is to diff files by hand, the fixture failed its demo. If a player learns off-screen stock without going there, the product's promise failed.

---

## 12. Client roadmap (shells, not products)

| When | Client | Notes |
|---|---|---|
| Now | CLI on loopback | The sitting |
| Later | Minimal web log and input | Same play door, after a fixture week |
| Later | PWA | Same play door |
| Optional | Discord bot | Same play door |
| Later | Tick-aligned 2D viewport | Click/tap → intent; interpolates only |
| Later | Voice | Speech-to-intent, TTS out |

Website is a skin. Do not start there. A prettier client does not make this the default AI RPG.

---

## 13. Acceptance criteria

### 13.1 Kernel shipped

A reviewer should treat the kernel as shipped only if all of these pass:

1. Load a pack from a folder.
2. One bundled one-shot pack plays in the CLI without editing code and is pleasant to leave running for a week. A host can tell the week happened. A player resume does not hand them off-screen stock. Walking to the shop is how they find the restock and the door note.
3. A second author can add a move to the pack without a kernel change.

### 13.2 Company gates (not the same checkbox)

The engineering gates can pass while the business is still a gist. Treat these as separate:

| Gate | Target | Meaning |
|---|---|---|
| External packs | 10 worlds we did not write | Format is usable |
| Lived-in shards | 3 shards survive 7 real days with ≥2 humans returning | Idle contract is a toy people keep |
| Paid renewal | ≥1 stranger renews a shard | Wedge is not theoretical |
| Cost | Idle shard-day cost measured and boringly low | Kernel-only path is real |
| Kill | People say “cool demo” and paste the prompt into ChatGPT | Stop |

Do not raise, brand, or marketplace before the first four trend the right way.

The next company step is a written list of 20 agent-native authors. Success is a world they wrote, running here, without a call.

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
- Coding agents need schemas and tools, not another GUI. MCP + pack is the 2026 distribution channel for engine work. Use `http://127.0.0.1:9738/mcp` from `spook run`. `--mcp-http` overrides that port. One GM seat, for playtesting and authoring loops; `spook ai-gm` is the shipped model GM.
- Memory and longevity are the unsolved axes in the 2026 field. The refusals aim there on purpose.

### 14.3 Why “AI NPCs from profiles” is not the business

That is table stakes (character cards, Voyage NPCs, etc.). The business is the **protocol plus the kernel that enforces it**: pack + firewall + tick + tests other runtimes can implement.

### 14.4 Paying wedge (one sentence)

Engine open source (AGPL-3.0); **always-on shard + lint/conformance paid**; host pays, friends free; BYOK for the live GM agent; kernel idle included.

Starting price to learn from, not a freeze: **$9 per shard-month** (band $8–15). Price like always-on hosting plus lint, not like Voyage/Fables inference tables. Measure cost per idle shard-day before treating $9 as real. Studio seats come after shards, not before.

### 14.5 Who pays (in order of alignment)

1. **Hosted playtest shards + conformance** — engine open source (AGPL-3.0); workspace, lint, always-on shard are paid (RPGJS / open-core pattern).
2. **Studio seats** — authors who already have packs.
3. **B2B kernel** — platforms that do not want to rewrite isolation + commit. Assume Voyage/Craft will not be this buyer.
4. **Pack registry** — tax verification (“this pack passed isolation on model X”), not creativity. Year-3. Nobody pays this until untrusted packs have burned someone.

Do not take 30% of someone else’s game. That is a platform company.

### 14.6 Unit economics constraint

A 4-player turn that spins a frontier model per NPC is a money fire. The idle path is kernel-only. Live path: one GM agent, small models for banter, kernel for numbers. Target to learn later: well under $1 per player-hour on mid models.

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

## 15. 10-year extension path

The core does not grow a second writer. Extensions are shells and packs.

| Horizon | Add |
|---|---|
| 0–1y | One beloved pack + first paid shards |
| 1–3y | Maker loop: invite link, other authors, lint/studio, a web skin |
| 3–5y | Optional tick-aligned 2D client (click → intent) |
| 5–7y | Hosting at more than hobby scale, registry, versioned rules packs |
| 7–10y | Format other clients speak — *or* we blink and become a content studio |

Later genre packs may override pause policy. The docks fixture does not.

**Not inherently text-only.** Text is the honest first client and the debug view. A later viewport only interpolates state the pack already has.

**Inherently limited to:** discrete ticks, budgeted agents, friend-scale shards, content supplied by packs and authors.

**Private OSRS analog (allowed reading):** tick machine + data-driven shard + friends host it.
**Private OSRS analog (forbidden reading):** we become Jagex.

---

## 16. Repo layout

```text
/README.md                 # how to build and play
/docs
  spec.md                  # referee law
  README.md                # points at spec.md
  product_spec.md          # this file; business notes, not law
/packs
  oneshot-docks/           # official fixture
/.grok
  skills/author-pack/      # pack-authoring skill
/crates
  kernel/                  # spookery-kernel
  round/                   # spookery-round
  spook/                   # package spook, binary spook, library spookery
    src/
      main.rs              # spook
      host/                # world loop, seats, and the idle clock
      gm_connection.rs     # shared GM-seat client
      protocol/            # shared lines
      terminal/            # spook play and spook gm
      ai/                  # spook ai-gm
    tests/
      promises.rs
      features/
```

Language: Rust. Three crates: spookery-kernel, spookery-round, and spook.

Agents implement from `docs/spec.md`. A human reviews. This file does not license a code change.

---

## 17. License

Decided October 9, 2026. legend-of-alan is the sole copyright holder and can offer separate commercial licenses.

- **Engine:** open source (AGPL-3.0). Runtime code, the tests of that code, and repository tooling and scripts are AGPL-3.0-or-later. Network use must share the corresponding source. The alternative of a use restriction on competing hosted shards is dropped.
- **Documentation, schemas, and packs:** CC BY 4.0, so a second implementer can speak the pack.
- **Brand:** Spookery / `spook` / pack. Do not brand as D&D.
- **Runtime language:** Rust.
- **Fixture rules:** tiny original ruleset in §8.1. Not SRD/ORC.

---

## 18. Review checklist for humans / agents

When reviewing this file, answer:

1. Is the refusal (“models do not write state”) still load-bearing in every section?
2. Did any paragraph smuggle in a website, MMO, or event-page editor as current work?
3. Does this file stay a business pitch, with referee law left in `docs/spec.md`?
4. Is the business still “format + runtime + studio,” not “our official campaign”?
5. Is the paying wedge still “shard + conformance,” or did a paragraph smuggle marketplace / official campaign / default-AI-RPG?
6. Is the bundled pack a world someone would leave running for a week, or only a secret-leak test double?
7. Is the kernel still the proof of the format (tests), not a closed World Engine and not a spec-only hope?

Suggested first read after review: `docs/spec.md`. This file is not the implementation task.
