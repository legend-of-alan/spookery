# Pack encoding law — v0

**Status:** Frozen for v0.  
**Companions:** `BRANDING.md`, `PRODUCT_SPEC_v0.5.md`, `STACK_v0.3.md`, `SHARD_TICK_SPEC_v0.3.md`, `NOW_AND_EFFECTS.md`.

`pack.yaml` and the `pack/` layout root replace `packet.yaml` / `packet/`. Field `pack_rev` replaces `packet_rev`. `world/` is still the canon-places subtree inside a pack.

This page is the format rule. Field shapes for `now.yaml` and effects live in `NOW_AND_EFFECTS.md`. JSON Schema files live in `/schema`. Tick behavior lives in `SHARD_TICK_SPEC_v0.3.md`.

## Three encodings, three jobs

| Encoding | Where | Authority |
|---|---|---|
| YAML | `pack.yaml`, `rules/*.yaml`, YAML frontmatter, `state/now.yaml`, `state/rumors.yaml` | Kernel-true data |
| Markdown body | After frontmatter in `world/**/*.md` and `cast/*.md` | Flavor only. Never mechanical |
| JSONL | `state/log/*.jsonl`, `state/memories/*.jsonl` | Append-only evidence of what the tick did |

JSON Schema validates the **data model**, not the file extension. MCP on the wire is JSON-RPC; that is the protocol, not a pack format.

Do not add TOML, JSON-per-place, or a database for canon or `Now`.

## Layout

```text
pack/
  pack.yaml                 # id, title, system, start, shard defaults
  world/
    places/<id>.md            # YAML frontmatter + optional prose
    factions/<id>.md
  cast/
    <npc>.md                  # starting profile (canon)
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

Canon (`world/`, `cast/`, `rules/`) is read-only at runtime.  
`state/` is the only live write surface.

## Frontmatter law

Entity files are Markdown with YAML frontmatter:

```markdown
---
id: mira
kind: npc
hidden: [fact_ledger_short]
facts:
  - id: fact_ledger_short
    text: The lamp-oil ledger is short.
    hidden: true
tick:
  eligible: true
  range: [docks_shop, docks_street, mira_rooms]
  stock_rate: { lamp_oil: 4 }
---

Mira keeps the shop neat and the tin out of sight.
```

Rules:

1. Delimiter is `---` on its own line at the start of the file. Body begins after the closing `---`.
2. If the tick would break when the body is deleted, the field was in the wrong place. Put it in YAML.
3. Bodies may be shown by `perceive` only as flavor attached to fact ids the viewer already may know. Bodies do not introduce places, items, flags, or facts.
4. **Hidden-string lint (fail closed):** if a body contains a string listed in any entity’s `hidden` text / hidden fact body, `spook lint` fails. Test 1 is allowed to be string-level.
5. Strip-bodies conformance: delete every Markdown body, rerun kernel tests. Firewall, stock, clocks, and travel graph must still pass.

## Who owns which sheet

| Thing | Canon | Live |
|---|---|---|
| Place, faction, NPC profile | `world/` / `cast/*.md` | Flags, stock, location, relationships in `state/now.yaml` |
| Player character | Starting template in `cast/` | Mutable sheet in `state/actors/<pc-id>.yaml` |
| World snapshot | — | `state/now.yaml` only |

Players never write `cast/` during play. A PC change that should become canon is a **proposal**.

## `now.yaml` is not hand-edited mid-shard

- The kernel mutates a typed `Now`. Disk YAML is a projection.
- Mid-shard hand edits are rejected on load unless passed through `spook migrate` / a studio commit that bumps `state_rev`.
- After a crash, `state_rev` on disk wins. A partial JSONL line without a matching commit is discarded.
- JSONL is evidence. It is not a second source of truth for live fields.

## Replay

1. Compare typed `Now` (debug).
2. Compare byte-equal `now.yaml` from the pinned emitter (ship gate).

Do not keep a parallel `now.canonical.json` unless the YAML emitter cannot be pinned. One snapshot.

## Ruleset files

`rules/moves.yaml` is the enum the kernel loads. Unknown moves reject.  
The official fixture uses the **tiny original** move list in the product spec, not an SRD/ORC subset. Other packs may exist later. v0 does not load two rulesets at once.

## Lint that must exist on day one

- Schema valid
- Frontmatter parses
- Strip-bodies still ticks
- Hidden strings absent from other actors’ views, memories, rumors, **and** from all Markdown bodies
- `now.yaml` writer is the runtime
- Player-facing resume / `perceive` never includes off-screen stock deltas (host `status` may)

## Idle digest vs player view (locked)

JSONL records every applied effect. Who may *read* those rows is not the same question.

| Reader | Filter |
|---|---|
| Host `spook status`, studio, conformance | All subsistence + plot rows |
| Player session resume | `message` to that actor; rumors now in their `known_by`; plot clocks they own |
| `perceive(as=player:id)` | Current place only: stock, door notes, present actors, public facts |

Stock, shop hours, and shop-door notes are place facts. They are not login facts.
