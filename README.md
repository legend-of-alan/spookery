<p align="center"><img src="assets/spookery-logo.png" alt="Spookery: Even ghosts play by the rules." width="720"></p>
<p align="center"><a href="https://github.com/legend-of-alan/spookery/actions/workflows/ci.yml"><img src="https://img.shields.io/github/actions/workflow/status/legend-of-alan/spookery/ci.yml?branch=main&label=CI" alt="CI"></a> <a href="https://github.com/legend-of-alan/spookery/releases"><img src="https://img.shields.io/github/v/release/legend-of-alan/spookery?include_prereleases&label=release" alt="Release"></a> <a href="#license"><img src="https://img.shields.io/badge/license-AGPL--3.0-blue" alt="License: AGPL-3.0"></a> <a href="https://github.com/sponsors/legend-of-alan"><img src="https://img.shields.io/badge/sponsor-%E2%99%A5-db61a2?logo=githubsponsors" alt="Sponsor"></a></p>

> Pre-alpha. Download a build from [Releases](https://github.com/legend-of-alan/spookery/releases) and expect things to change.

Spookery is a text RPG engine for shared, living worlds. Let a model run an RPG on its own and the world slips. An NPC forgets what it said. A roll gets fudged. A locked door opens anyway.

Creativity isn't utter discord, it's controlled chaos.

You write a world as a pack of plain YAML rules and Markdown pages for places and people. Coding agents like Claude Code and Cursor can build one with `spook new` and `spook check`. Then you host it and sit down with up to three players and a GM.

Bring a friend to GM or bring your own model key. Either way, the engine is the only thing that changes the world. The GM tells the story and asks the engine to commit.

- ⚙️ Play moves in rounds. Everyone queues a move, and the engine resolves them.
- 🎲 Dice are seeded. A d20 check can partly succeed, and NPCs roll their own checks.
- 📙 Every commit goes in a hash-chained journal. You can replay a game and check every roll.
- ⚡ Triggers fire for in-round changes.
- 🕰️ A world clock keeps time, and NPCs keep habits, like restocking the shop.
- 🙈 Players see whether they succeeded, and only the GM sees the dice.

New words like seat and tick are explained in the [glossary](docs/README.md#glossary).

## Play

Grab the archive for your machine from [Releases](https://github.com/legend-of-alan/spookery/releases):

- Linux: `x86_64-unknown-linux-gnu`
- Apple silicon Mac: `aarch64-apple-darwin`
- Intel Mac: `x86_64-apple-darwin`
- Windows: `x86_64-pc-windows-msvc`

Unpack it and open a terminal in that folder. Copy the demo, then start the host. You don't need a GM or a model key.

```sh
cp -r packs/demo-d20 my-demo
./spook run my-demo
```

> NOTE: The builds are unsigned. On a Mac you may need `xattr -d com.apple.quarantine spook` once. Windows may say More info, then Run anyway.

Leave that running. Open a second terminal in the same folder, join, and pick Corin.

```sh
./spook play
```

You're in the Yard. Daylight is running out. Press Ctrl+C in the host terminal when you're done. It saves and quits.

Want a model to run the GM seat? Set three env vars and run `spook ai-gm`. See [Model GM](docs/README.md#model-gm).

## Build from source

You need [rustup](https://rustup.rs). It installs the Rust version pinned in `rust-toolchain.toml`.

```sh
git clone https://github.com/legend-of-alan/spookery.git
cd spookery
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
```

That gives you one binary, `target/release/spook`. The demo pack is at `packs/demo-d20`.

## At the table

Type `help` any time. It lists the commands you can type here and now, and what each one can name, as `travel <exit>: gate, cart, Loft, Shed`. `map` lists the places you have visited, with their exits.

| Type | What it does |
| --- | --- |
| `look` | Shows the place again |
| `say <words>` | Talks. `say Mira hi` (or `tell Mira hi`) talks to Mira. Talking takes no game time |
| `travel <exit>` | Walks to another place |
| `take <item>` | Picks something up |

> NOTE: `help`, `map`, `look` and `say` work in every pack. The other verbs come from the pack, so `travel` and `take` are the demo's.

## The packs

`packs/demo-d20` is the one to play first. Each release bundles it. It's a short demo in a small yard. Corin can best Briar with a cudgel and go through the gate. Or he can banish a goblin for its nail and take the cart, setting the wheel with `ward` or with Oden's help through `brace`. Corin loses if he is knocked out, or if daylight runs out before he reaches the lane.

> NOTE: CI checks that it loads. No test plays it, so its story can change without breaking a test.

Test packs live in `tests/packs`. `tests/packs/docks` is the one the engine and seat tests use. It has three player characters at the docks, and a yard behind the street with a fight, a d20 strike beside a 2d6 check, and a lane that ends the story.

> NOTE: Test packs are fixtures, not games.

## Other commands

| Command | What it does |
| --- | --- |
| `spook new <dir>` | Writes a starter pack you can edit |
| `spook check <pack>` | Reports pack problems without writing anything |
| `spook verify <pack>` | Replays the journal and checks that it rebuilds the save |
| `spook gm` | Sits you in the GM seat |
| `spook ai-gm` | Puts a model in the GM seat |

Run `spook --help` for the full list.

## Docs

- [docs/README.md](docs/README.md) is the index, with the glossary and host flags.
- [docs/first-pack.md](docs/first-pack.md) walks you through making a pack.
- [docs/pack-reference.md](docs/pack-reference.md) lists every pack field.
- [docs/engine-guarantees.md](docs/engine-guarantees.md) says what the engine promises.
- [docs/seat-protocol.md](docs/seat-protocol.md) is the seat protocol for clients and GMs.
- [CONTRIBUTING.md](CONTRIBUTING.md) covers issues, tests and hooking up coding agents.

## License

The engine is AGPL-3.0-or-later, and the crates a game client needs are MIT. The full split is in [docs/README.md](docs/README.md#license). If AGPL doesn't fit your project, open an issue and we can talk.
