# Docs

Start with the [README](../README.md) to build and play. This page points at everything else.

| Doc | What it's for |
| --- | --- |
| [Engine guarantees](engine-guarantees.md) | What the engine promises. The feature files test it, and they win if the two disagree. |
| [Write your first pack](first-pack.md) | A walkthrough that starts from `spook new` and makes your first edits. |
| [Pack reference](pack-reference.md) | Every pack field, for people and agents writing packs. |
| [Expressions](spec.md) | The small number language pack fields use, like `2d6` or `self.might`. |
| [Seat protocol](seat-protocol.md) | The wire format, for anyone writing a player client or a GM. |
| [CONTRIBUTING](../CONTRIBUTING.md) | Issue rules, running the tests, and pointing a coding agent at the table. |

Further down this page you'll find the [glossary](#glossary), [host options](#at-the-table), [GM setup](#game-master), [what the model sees](#privacy) and the [license](#license).

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
| **Spookery** | The project name. The library is `spookery`. The package and binary are `spook`. |
| **Pack** | A folder of text that is the world: canon, rules and current state. |
| **Seat** | One place at the table. Each player character is a seat, and the GM is one more. |
| **Canon** | Files under `world/`, `cast/` and `rules/` in a pack. They don't change while the world runs. |
| **`spook`** | The program that loads a pack. It is the only thing that writes the pack's state. |
| **Kernel** | The code that decides what is legal, rolls checks and tracks who knows what. |
| **Tick** | A unit of game time on the world clock. Talking takes no ticks. Moving and acting do. |
| **Round** | One batch in which queued intents are settled together and the results are logged. |
| **Intent** | A proposed move from a player, an NPC or the engine's scheduler. |
| **Effect** | A checked change to the world, produced from an intent. |
| **Expression** | A number in a rule. An integer, dice such as `2d6` or `2d20kh1`, a meter read such as `self.might`, and `+`, `-`, `*`, `/`, `min`, and `max`. A condition's value does not roll dice. |
| **GM** | The storyteller seat. Usually an agent that talks to the agent door. `spook ai-gm` is the built-in model GM. |
| **Play door** | The human client, `spook play`, over loopback. |
| **Agent door** | The MCP endpoint `spook run` serves, `http://127.0.0.1:9738/mcp` by default. |

## Repo layout

```text
README.md              how to build and play
docs/                  guarantees, first-pack guide, pack reference, expressions, seat protocol, this file
packs/                 sample-d20 is the starter. oneshot-docks is the multi-PC fixture
.grok/skills/          the author-pack skill
crates/
  protocol/            spookery-protocol: player and GM wire codecs
  engine/              spookery-engine: world model, rules, rounds, and journal
  agent/               spookery-agent: model GM turn state machine
  cli/                 spookery-cli: spook play, spook gm, and spook ai-gm
  server/              spookery-server: world loop, seats, idle clock, and MCP door
  spook/               package and binary spook, library spookery
    tests/features/    BDD scenarios
```

## At the table

These are the host options and the commands that touch a save.

| Option or command | What it does |
| --- | --- |
| `--addr <addr>` | Sets the game address. The default is `127.0.0.1:9737`, and plain `127.0.0.1` means that same port. Only loopback is allowed. Anything else is refused before the pack opens. |
| `--mcp-http 127.0.0.1:<port>` | Sets the agent door. By default it's the game port plus one, so `127.0.0.1:9738`. |
| `--as <id>` | Picks your character in `spook play`. `pc_rowan` and `player:pc_rowan` mean the same seat. |
| `spook status <pack>` | Prints the host digest. It doesn't write the pack. |
| `spook tick <pack>` | Advances the world one tick and writes `state/`. |
| `spook migrate <pack>` | Stores the current canon hash in the save. |

Stop `spook run` before you use `tick` or `migrate`, because the running server keeps the save in memory.

If you join as a character who's already seated, `spook play` exits 1 with `That character is already taken. [SEAT_TAKEN]`. A fourth player gets `The table is full. [SESSION_FULL]`.

`spook` builds with both the `server` and `client` features by default. To build just one, pass `--no-default-features --features server` or `--no-default-features --features client`.

## Game master

### No GM

You can leave the GM seat empty. Then nobody narrates and no model gets called. Speech goes on the tape without moving the clock, and each move commits as its own tick.

If a move leaves every seated player busy, the clock runs on until they arrive.

### Model GM

`spook ai-gm` puts a model in the GM seat. It doesn't start the host, so run `spook run` first.

It posts to `{SPOOK_GM_URL}/chat/completions` with one tool, `act`. The model answers by calling that tool, and then the engine commits the queued lines.

You need all three of these. There's no default URL.

| Variable | What it holds |
| --- | --- |
| `SPOOK_GM_KEY` | Your API key, sent as a bearer token. It's read from the environment, never from the pack. |
| `SPOOK_GM_URL` | The endpoint origin, including the `/v1` prefix your provider documents. |
| `SPOOK_GM_MODEL` | The model name to send. |

If one is missing, `spook ai-gm` prints `<NAME> is unset` and exits 1 before it connects. It checks them in the order shown in the table.

For xAI:

```sh
SPOOK_GM_KEY=your-key SPOOK_GM_URL=https://api.x.ai/v1 SPOOK_GM_MODEL=grok-4.20-0309-non-reasoning spook ai-gm
```

For Claude, through its chat-completions endpoint. Swap in whichever Claude model you want.

```sh
SPOOK_GM_KEY=your-key SPOOK_GM_URL=https://api.anthropic.com/v1 SPOOK_GM_MODEL=claude-sonnet-4-5 spook ai-gm
```

For [Ollama](https://ollama.com) on your own machine:

```sh
ollama pull qwen2.5:0.5b
SPOOK_GM_KEY=ollama SPOOK_GM_URL=http://127.0.0.1:11434/v1 SPOOK_GM_MODEL=qwen2.5:0.5b spook ai-gm
```

If the provider refuses a call, stderr shows `model call returned <status>: <text>`. The text is the provider's error on one line, cut to 200 characters, with your key replaced by `[key]`.

`crates/spook/tests/integration/live_round.rs` plays one real round against whatever those variables point at. It's ignored by default. CI runs it as described in [Live AI CI](../CONTRIBUTING.md#live-ai-ci).

### Human GM

`spook gm` puts you in the same GM seat. Run `spook run` first, then:

```sh
spook gm
```

Type `help` to see the GM commands. `spook gm --raw` prints each protocol frame as one JSON line instead.

Both `spook gm` and `spook ai-gm` connect to the agent door. Use `--mcp-http 127.0.0.1:<port>` to point them somewhere else. If you started the host with `--addr 127.0.0.1:0`, the OS picks the ports and the listening line tells you the door. Pass that to `--mcp-http`.

## Privacy

`spook ai-gm` sends table text to whatever host `SPOOK_GM_URL` names. There's no default host. The key comes from `SPOOK_GM_KEY` in your environment and never lives in the pack.

Each call is one chat-completions POST. The body has the model name, `"stream": false`, the `act` tool and one user message.

That message is a bounded JSON prompt. It holds the last 16 says this process has handled, the new player lines, the move menu, the queued intents and trimmed player and NPC views. After a roll, a follow-up call adds the dice faces.

The views are trimmed on purpose. A player view that's only a room census drops known facts, messages and unknown mentions. An NPC view keeps each fact's id and text. Fact bodies are always dropped.

The prompt does include the Markdown body of the current place and of each cast member in those views. Each body is cut at 800 characters.

It never sends `save.yaml` or the tape file. Those stay on disk in the pack. A new `spook ai-gm` process starts fresh and remembers nothing from the last one.

## License

The engine is open source under AGPL-3.0. Runtime code, its tests, and the repo's tooling and scripts are AGPL-3.0-or-later.

The `spookery-protocol` crate is MIT, so a game client can reuse the wire codec freely. Docs, schemas and packs are CC BY 4.0.

The full terms are in [LICENSE](../LICENSE), [LICENSE-MIT](../LICENSE-MIT) and [LICENSE-CC-BY-4.0](../LICENSE-CC-BY-4.0).
