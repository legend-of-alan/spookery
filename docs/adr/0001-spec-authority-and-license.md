# ADR 0001: Spec authority and license

- Status: Accepted
- Date: 2026-10-02
- Source: Headers of `docs/architecture_v0.md`, `docs/architecture_v0.1.md`, and `docs/architecture_v0.2.md` (through line 8 of each)

## Context

`docs/architecture_v0.md` states "**Status:** Frozen normative spec for v0. Do not edit it to describe the sitting. The v0.1 sitting is specified in `docs/architecture_v0.1.md`, and where that file names a change, it wins. Every rule it does not name still holds here." Its audience line is "**Audience:** Founders, a reviewing programmer, and coding agents implementing crate by crate." It states "This file is the normative spec for v0."

`docs/architecture_v0.1.md` states "Status: Frozen. Normative spec for the v0.1 sitting. Gates A–E in §9 are proved. Where this file names a change, it wins over `docs/architecture_v0.md`. Every v0 rule this file does not name still holds. A rule written in neither file is out of scope. §9 does not open another gate." It states "This file is the frozen spec for the sitting. `docs/architecture_v0.md` remains the frozen spec for the kernel rules this file does not change. This file does not copy it. §9 records the order in which the sitting became authority."

`docs/architecture_v0.2.md` states "Status: Draft." It states "This file is the draft spec for the v0.2 minor. `docs/architecture_v0.1.md` and `docs/architecture_v0.md` stay frozen. This file does not copy them. Do not edit those files."

## Decision

"Where this file names a change, it wins over `docs/architecture_v0.1.md`, then `docs/architecture_v0.md`. Where this file does not name a change, v0.1 holds, then v0. A rule written in none of them is out of scope."

Spec text is CC BY 4.0. `docs/architecture_v0.md` states "**License:** CC BY 4.0 (spec). Runtime code that lands under `/src` is AGPL-3.0-or-later." `docs/architecture_v0.1.md` and `docs/architecture_v0.2.md` each state "License: CC BY 4.0, same as the v0 spec. Runtime code under /src stays AGPL-3.0-or-later."

`docs/architecture_v0.md` and `docs/architecture_v0.1.md` stay frozen. `docs/architecture_v0.2.md` states "Do not edit those files." `docs/architecture_v0.md` states "Do not edit it to describe the sitting." `docs/architecture_v0.1.md` states "§9 does not open another gate."

## Consequences

"A rule written in none of them is out of scope." `docs/architecture_v0.md` states "Do not edit it to describe the sitting" and "`product_spec.md` is non-normative business notes." `docs/architecture_v0.2.md` states "This file does not copy them."

## No existing proof

"The demo below is true. The bare tag `v0.2` is cut on patch `v0.2.6`. Patch `v0.2.0` is tag `v0.2.0` at `913eeb6`. Patch `v0.2.1` is tag `v0.2.1`. Patch `v0.2.2` is proved. Patch `v0.2.3` is proved. Patch `v0.2.4` is proved. Patch `v0.2.5` is proved. Patch `v0.2.6` is proved. A later patch is tagged `v0.2.N` when that patch's proofs are green."

"v0.2.2 is the first edit of `packs/oneshot-docks`."

"A session reads this file and implements the open patch. A pin this file quotes is the pin the session uses."

## Superseded

`docs/architecture_v0.md` states "A rule that is not written here is out of v0 unless `docs/architecture_v0.1.md` names the change." `docs/architecture_v0.1.md` states "A rule written in neither file is out of scope." The header of `docs/architecture_v0.2.md` replaces both: "Where this file names a change, it wins over `docs/architecture_v0.1.md`, then `docs/architecture_v0.md`. Where this file does not name a change, v0.1 holds, then v0. A rule written in none of them is out of scope."
