# ADR 0002: One writer of state

- Status: Accepted
- Date: 2026-10-02
- Source: docs/architecture_v0.md §1, lines 47–72

## Context

The pack is the world. The kernel is the proof that the pack can be refereed. The spec is the refusal of a second writer. v0 ships all three as one repository. It does not ship a campaign, a table product, or a model.

## Decision

Humans and models submit intents. The only durable writers of `Now` are `tick_commit` and `commit_turn`. `apply_effect` mutates an in-memory transaction and is not a writer. A crash after `apply_effect` and before `tick_commit` leaves the previous `state_rev` on disk.

Persist writes `state/` via a temp file, `fsync`, and an atomic rename. Studio `patch_entity` writes canon under `world/` or `cast/` and never `state/`. Models never patch `state/`, `world/`, or `cast/`. Inbound `llm_say` is discarded. Persisted intents store `llm_say: null`.

Markdown bodies are flavor. Every number the tick uses is a YAML field or a Rust constant. A pack with every Markdown body deleted must still tick.

There is no GM perceive on the tick process. Host omniscience is read-only `spook status` with `viewer = host`. The one `as=gm` read is table-profile `recall` (pin P24). `ViewerId` has no `Gm` variant.

## Consequences

- Feature `v0_atomic_commit` already asserts the crash window.
- Feature `v0_strip_bodies` already asserts that deleted Markdown bodies still tick.
- Feature `v0_idle_ux` already asserts that host status and player resume are different views.
- Clients in this repo do not open `now.yaml` for write.

## No existing proof

None added here. The stage table's rows that no feature card names stay in this record.

## Superseded

None inside §1. v0.1 attaches seats to the same writer rule and says so in ADR 0008.
