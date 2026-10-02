# ADR 0011: Host is not shard behavior

- Status: Accepted
- Date: 2026-10-02
- Source: `docs/architecture_v0.2.md` Host, line 191

## Context

"A Grok skill may tell an already-running turn to call `say` and `session_status`. A host monitor on the tape may print new `player:` lines to wake that turn."

## Decision

"Neither is shard behavior. None of these patches writes the skill."

A Grok skill may tell an already-running turn to call `say` and `session_status`. A host monitor on the tape may print new `player:` lines to wake that turn.

## Consequences

"Neither is shard behavior." The skill and the host monitor are not shard behavior. "None of these patches writes the skill." This record is not a scenario.

## No existing proof

There is no proof function for the skill. The sentences that name the observable are "A Grok skill may tell an already-running turn to call `say` and `session_status`. A host monitor on the tape may print new `player:` lines to wake that turn."

## Superseded

None. The Host section does not name a later file that replaces these sentences.
