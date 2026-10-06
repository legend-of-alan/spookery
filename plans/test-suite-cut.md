# Test suite cut

| | |
|---|---|
| Author | legend-of-alan |
| Date | 2026-10-05 |
| Status | Done |
| Repo | `C:\Users\alana\Local\spookery` |

The suite keeps the world outcome of each design rule. It drops proofs that repeat another proof, frozen bytes, and refusal-token spellings. `docs/spec.md` stays as it is.

## 1. Delete the strict duplicates

Remove `tests/features/harness/status_writes_nothing.feature`, the seated `talk` scenario, the knife-with-GM scenario, and the 100-tick ledger soak.

Status, seated speech, the missing knife, and the ledger absence are already proved by the digest scenario, the seated `say`, the no-GM knife mark, and the lint perceive.

## 2. Delete the frozen snapshots

Remove the four golden tick lines, the 166-byte emitter sample, and the docks `now.yaml` length and sheet hash. Leave the docks load/emit round-trip.

Those pins break when a harmless byte changes. The wait, the budget bound, the empty ticks, the catch-up cap, and the round-trip already prove the behaviors.

## 3. Delete the refusal-only scenarios

Remove `tests/features/seats/gm_without_stdio_refuses.feature`, the `pack-gm` scenario, `include_tick false`, the agenda-name scenario, and four of the five loopback rejections. Keep the open-address rejection beside the two loopback accepts.

Each deleted scenario only checks that stderr is `NOT_IN_SPEC`. The three menus, the held round, and the paused debt clock already carry the design.

## 4. Drop the token, keep the outcome

On the scenarios that stay, stop asserting `SESSION_FULL`, `MISSING_ITEM`, `TABLE_EMPTY`, `LLM_DISABLED`, `UNSCOPED_PERCEIVE`, `PACK_REV_MISMATCH`, and `OUT_OF_RANGE`.

The design is the unchanged seat, the tick that does not start, the missing tape, the unchanged save, the scoped perceive, the tick that writes nothing, and Rowan staying in the shop.

## 5. Collapse the leftover doubles

Merge the two `SCHEMA` load cases into one closed moves load, delete `tests/fixture/canon_schemas.rs`, and move the `src/schedule.rs` no-spawn scan onto the boundary feature so the timed wake can go.

The loader already rejects an unknown moves key. The kernel never reads `schema/*.json`. The timed wake repeats the 12-tick catch-up. The spawn scan is the part the import scan does not cover, because `src/schedule.rs` lives in the bin.
