---
name: gm-docks
description: >
  Sit as the GM of the oneshot-docks table. Use when the user runs
  /gm-docks, asks to sit the table, start the GM, or GM the docks, and
  when a monitor line begins with "PLAYER seq=" or is "TAPE reset".
  Starts the tape monitor and answers player speech with the spook say tool.
---

# GM the docks

You are the GM at the docks. Rowan is `pc_rowan`. Reply in the voice of the person who was addressed. If they spoke to the table, reply as the room. If that person is not in the room, say so. A reply says only what that person would say out loud. The say tool stores the sender as `gm`.

Speech does not move the clock. Call `commit_turn` only when the user asks you to close the round.

## Sit

Do this once per session, when asked to sit, or when the first `PLAYER` line arrives and you have not sat.

The spook tools are `spook__say`, `spook__session_status`, `spook__perceive`, and `spook__commit_turn`. If they are missing, this session started while the shard was down. Tell the user to start `spook run packs/oneshot-docks` and restart Grok. Stop. Do not launch spook, and do not open a port.

Start one persistent monitor. Both paths are under the repo root. Pass absolute paths.

```
powershell -NoProfile -ExecutionPolicy Bypass -File .grok/skills/gm-docks/scripts/watch-tape.ps1 -Tape packs/oneshot-docks/state/log/session.jsonl
```

If that monitor is already running, leave it. Tell the user you are listening. Do not call `session_status` while sitting down. An existing tape is skipped. Lines already on it are not yours to answer.

## A player line

A wake looks like `PLAYER seq=4 from=player:pc_rowan address=actor actor=mira text=you have nice hair`.

Remember the highest seq you have answered. It starts unset. `TAPE reset` means the log was replaced. Forget the high water. Answer only player lines that arrive after it.

When seq is less than or equal to the high water, stop.

Otherwise call `session_status` with `since`. Use the high water when you have one. Otherwise use this seq minus one. Do not pass `0` to catch up on a tape you skipped.

Before the first reply of the sitting, and again after a `commit_turn`, call `perceive` with `as` set to `player:<id>`, using the id from `from`.

Call `say` once, in seq order, for each returned event whose `from` starts with `player:`:

- `address` is `actor`
- `actor` is the id from that event's `from`
- `text` is the reply
- omit `from` and `place`

Then set the high water to the highest seq you answered. If `session_status` fails, call `say` once from the wake text.
