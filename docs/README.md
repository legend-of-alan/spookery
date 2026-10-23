# Docs

[Engine guarantees](engine-guarantees.md) states what the engine promises. The feature files test them. Where the doc and the feature files disagree, the feature files win.

[Write your first pack](first-pack.md) walks through `spook new` and the first edits. [Pack reference](pack-reference.md) is the field reference for authors and agents. [Expressions](spec.md) is the number language those fields share. [Seat protocol](seat-protocol.md) is the wire for a player client or a GM integration.

Do not restore `docs/legacy_architecture/` or `docs/adr/`.

## Principles

1. **Text is the source of truth.** If people and models can't diff it, it doesn't belong in the product.
2. **Numbers live in code.** Dice and outcomes come from the engine. Models narrate after the tools return.
3. **Idle worlds are boring on purpose.** Shops keep living while the table is away, but the plot doesn't finish itself.
4. **Clients are swappable.** A terminal, Discord, voice or graphics client is a shell, not a second product.
5. **The engine proves the format.** A feature that can't be written as a pack field isn't a feature.

## How this differs

The model does not keep the world. It can talk, and it can ask the engine to commit a round. Dice, items, and places change only inside that commit. The journal records the round. Replay repeats the recorded rolls. `spook verify` checks the journal. A player sees a hidden fact only when that character knows it.

## Glossary

| Term | Meaning |
|---|---|
| **Spookery** | The project name. The library is `spookery`; the package and binary are `spook`. |
| **Pack** | A folder of text that is the world: canon, rules and current state. |
| **Canon** | Files under `world/`, `cast/` and `rules/` in a pack. They don't change while the world runs. |
| **`spook`** | The program that loads a pack. It is the only thing that writes the pack's state. |
| **Kernel** | The code that decides what is legal, rolls checks and tracks who knows what. |
| **Tick** | A unit of game time on the world clock. |
| **Round** | One batch in which queued intents are settled together and the results are logged. |
| **Intent** | A proposed move from a player, an NPC or the engine's scheduler. |
| **Effect** | A checked change to the world, produced from an intent. |
| **Expression** | A number in a rule. An integer, dice such as `2d6` or `2d20kh1`, a meter read such as `self.might`, and `+`, `-`, `*`, `/`, `min`, and `max`. A condition's value does not roll dice. |
| **GM** | The storyteller seat. Usually an agent that talks to the agent door; `spook ai-gm` is the built-in model GM. |
| **Play door** | The human client, `spook play`, over loopback. |
| **Agent door** | The MCP endpoint `spook run` serves, `http://127.0.0.1:9738/mcp` by default. |

## Repo layout

```text
README.md              how to build and play
docs/                  guarantees, first-pack guide, pack reference, expressions, seat protocol, this file
packs/                 sample packs (oneshot-docks is the official fixture)
.grok/skills/          the author-pack skill
crates/
  protocol/            spookery-protocol: player and GM wire codecs
  engine/              spookery-engine: world model, rules, rounds, and journal
  spook/               package and binary spook, library spookery
    src/host/          world loop, seats and the idle clock
    src/terminal/      spook play and spook gm
    src/ai/            spook ai-gm
    tests/features/    BDD scenarios
```
