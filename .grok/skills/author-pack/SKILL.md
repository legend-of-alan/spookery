---
name: author-pack
description: >
  Edit a Spookery pack and check it with spook check --json. Use when
  asked to author a pack, add a move, meter, check, or habit, or make
  a pack pass spook check.
---

# Author a pack

`spook check` is the source of truth for file formats. This skill does not list keys. Unknown keys are rejected, so let check tell you what is wrong.

## Start

- New pack: `spook new <dir>`. To change an existing pack, copy it first and edit the copy.
- Leave `packs/oneshot-docks` as it is. `spook run` writes `state/` into the pack you pass.
- File shapes: `docs/pack-reference.md`. `spook check` reports any unknown or missing key. Walkthrough: `docs/first-pack.md`. Behaviour: `docs/engine-guarantees.md`.

## The loop

1. Edit the copy.
2. Run `spook check --json <pack>`. stdout is one JSON array. Each object has `file`, `line`, `column`, `path`, `code`, `message`, and `hint`.
3. Fix each object. Follow its `hint` and use `code`. The message text may change.
4. Repeat until stdout is `[]` and the exit code is 0.
5. Run `spook status <pack>` to print the host digest.

## What check cannot teach

- `state/save.yaml` is the save. Don't hand-edit its keys. `pack_rev` in it is a hash of `world/`, `cast/`, and `rules/`, and only `spook migrate <pack>` stores a new one.
- After you edit anything under `world/`, `cast/`, or `rules/`, check reports `PACK_REV_MISMATCH` until you run `spook migrate <pack>`, then check again. Editing `pack.yaml` does not, because it is not hashed. Migrate does not fix a shape error.

## Worked example

On a copy of `packs/oneshot-docks`, append this rule to the `rules` list in `rules/verbs.yaml`:

```yaml
  - id: scan_the_street
    when:
      verb: scan_the_street
    do: []
```

Check reports `PACK_REV_MISMATCH`. Run `spook migrate <pack>`, then `spook check --json <pack>`. stdout is `[]`.
