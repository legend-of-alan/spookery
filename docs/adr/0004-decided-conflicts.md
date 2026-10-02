# ADR 0004: Decided conflicts

- Status: Accepted
- Date: 2026-10-02
- Source: `docs/architecture_v0.md` lines 28–43, the C1–C10 table

## Context

"The rows below are decisions the earlier drafts stated in more than one way."

C1: "Tick §8.1: tick `perceive` requires `as=npc:<id>` and forbids `as=gm`. Tick §11 lets a live GM call `recall` as a player or as GM. Product §10.2 adds "or system actor". Product principle 3 allows only `player:<id>` and `npc:<id>` on `perceive`."

C2: "Product criterion 2 says `perceive(as=npc:mira)` never contains Mira's hidden strings. Tick test 1 forbids that string in Aldric's memories, rumors, and `perceive(as=npc:aldric)`. The `Now` example gives Mira `known_facts: [fact_ledger_short]`, and tick §8.3 returns `known_facts` as ids plus short text."

C3: "Tick §8.1 says `recall` is "keyword/semantic". Stack says keyword plus fact-id index, and lists an embedder as out of v0."

C4: "Tick §12 once illustrated `seed` as `shard:abc|tick:1842`. The stack formula is `shard_id|tick_index`."

C5: "Tick §5.2 prints `ceil(D * 60 / tick_every_real_seconds)` while `D` is real hours and `tick_every_real_seconds` is seconds."

C6: "`state_rev` is specified as +1 per successful `tick_commit`. `commit_turn` is also a `state/` writer."

C7: "Product §9 shows `spook ./pack`. Stack names the subcommand `run`."

C8: "Product §15 allows an optional ugly `--serve` after a week-idle fixture, inside days 71–90. The planning brief says not to schedule `--serve`."

C9: "The `Now` example rumor `r_184` carries hidden `fact_ledger_short` and lists Aldric in `known_by`. Tick test 1 says Aldric does not know Mira's hidden fact."

C10: "Product §10's opening sentence says "two servers or two tool profiles". The same section and the stack name three profiles."

## Decision

"Implement the Rule column."

C1: "Tick profile `perceive` and `recall` accept only `as=npc:<id>`. Table and CLI `perceive` accept `as=player:<id>` or `as=npc:<id>`. `as=gm` on `perceive`, and on every tick-profile tool, is `UNSCOPED_PERCEIVE`. There is no system viewer on `perceive`. Table `recall` accepts `as=gm` and returns every fact id and its text (pin P24). `ViewerId` has no `Gm` variant."

C2: "Test 1 is the suite. A viewer receives a fact's text only when that fact id is in their `known_facts`, or the fact is public and at their place. Mira can perceive a hidden fact she knows. Criterion 2's literal reading is not implemented."

C3: "`recall` is keyword match plus fact-id lookup inside the viewer's allowed facts. No vector index."

C4: "Closed. `seed` stores `{shard_id}|{tick_index}`. Tick §12's example is `abc|1842`. One function, `seed_preimage`, is the only formatter. `rng` is one BLAKE2s-256 of that string."

C5: "Implement the printed formula. Do not silently substitute `D * 3600`. Also apply the §3.1 world-hour cap. The burst stops at the tighter bound. Flagged in §7."

C6: "Both successful commits increment `state_rev`. `tick_index` increments only when the tick loop runs. See §16."

C7: "The only load invocation is `spook run <pack>`. See §11 and §16."

C8: "Gate C does not include `--serve`. A later edge would be the same binary and the same kernel. This document specifies no HTTP route. Day numbers in `product_spec.md` are a pace sketch and lose to §14."

C9: "The example is a shape, not fixture start state. The fixture rumor cites a public fact. Aldric's `known_facts` omit `fact_ledger_short`."

C10: "One process, three profiles, one profile bound for the life of the process."

## Consequences

The Rule column is what a session implements. C2's automated assert is feature F-v0-secret. C9 names that same Test 1: Aldric does not know Mira's hidden fact.

C1 excludes a system viewer and a `Gm` variant of `ViewerId`. `as=gm` on `perceive`, and on every tick-profile tool, is `UNSCOPED_PERCEIVE`. C3 excludes a vector index. C5 excludes a silent substitution of `D * 3600`. C7 excludes `spook ./pack`. C8 excludes `--serve` from Gate C and specifies no HTTP route. C10 excludes a two-profile process.

## No existing proof

This record adds no scenario for a Rule. C2's assert is feature F-v0-secret. The Rule sentences for C1 and for C3 through C10 stay quoted under Decision.

## Superseded

C8 states "Day numbers in `product_spec.md` are a pace sketch and lose to §14." The later text is `docs/architecture_v0.md` §14, the gates. "Gate C does not include `--serve`." Product §15's optional `--serve` inside days 71–90 loses to that section.
