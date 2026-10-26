<p align="center"><img src="assets/spookery-logo.png" alt="Spookery: Even ghosts play by the rules." width="720"></p>

> Early work in progress, not a playable release yet.

Spookery is a table engine for a living world. An author writes a pack, a folder of YAML rules plus Markdown for places and people, and hosts it with `spook run`. One GM seat and up to three players take actions. The engine is the only writer of world state. Dice are seeded, and a replay repeats the recorded rolls. `spook verify` checks the journal against that record.

A model can sit the GM seat. It talks, and it asks the engine to commit. It does not edit the save. The journal is the audit trail: every commit is in the log, and verify replays it.

The glossary is in [docs/README.md](docs/README.md#glossary).

`packs/sample-d20` is the small starter you play first.

`packs/oneshot-docks` is the multi-PC fixture the engine and seat tests use.

## Build

You need Rust 1.89 or newer from [rustup](https://rustup.rs). On Linux you also need `pkg-config` and `libssl-dev` (on Debian or Ubuntu: `sudo apt install pkg-config libssl-dev`).

```sh
git clone https://github.com/legend-of-alan/spookery.git
cd spookery
cargo build --release --locked
export PATH="$PWD/target/release:$PATH"
```

The binary is `target/release/spook`. `spook` enables the `server` and `client` Cargo features by default. Pass `--no-default-features --features server` or `--no-default-features --features client` to build one slice.

## Start

`spook run` writes its save into the pack folder, so always play a copy, never the folder inside the repo. You are Lark, in the Gate Yard. A cart blocks the lane until you chock its wheel, and Briar bars it until you ask.

```sh
cp -r packs/sample-d20 /tmp/yard
spook run /tmp/yard
```

The host prints one line, `{"kind":"listening","game":"127.0.0.1:9737","door":"127.0.0.1:9738"}`, and waits. In another terminal, from the repo folder and with the same `PATH`:

```sh
spook play
```

With no GM seated, each line commits on its own. Press Ctrl+C in the host terminal when you stop. Then:

```sh
spook verify /tmp/yard
```

`verify` prints `verified <n> records, state_rev <r>` when the journal matches. See [Prove the log](CONTRIBUTING.md#prove-the-log).

## At the table

`help` lists the verbs and the exits. `look` prints the place again. `say` plus the words after it is speech, and it does not tick. Anything the table doesn't know tells you to type help.

The other host commands are in [docs/README.md](docs/README.md#at-the-table). A person sits the GM seat with `spook gm`. A model sits it with `spook ai-gm`. Setup for both is in [docs/README.md](docs/README.md#game-master).

## Docs

- [docs/engine-guarantees.md](docs/engine-guarantees.md) states what the engine promises. The feature files test them. Where the doc and the feature files disagree, the feature files win.
- [docs/README.md](docs/README.md) has the principles, glossary and repo layout.
- [docs/first-pack.md](docs/first-pack.md) walks through `spook new`. [docs/pack-reference.md](docs/pack-reference.md) is the field reference.
- [docs/spec.md](docs/spec.md) is the number language. [docs/seat-protocol.md](docs/seat-protocol.md) is the wire.
- [CONTRIBUTING.md](CONTRIBUTING.md) has issue and PR conventions, and the [agent clients](CONTRIBUTING.md#coding-agent-clients).
- [LICENSE](LICENSE), [LICENSE-MIT](LICENSE-MIT) and [LICENSE-CC-BY-4.0](LICENSE-CC-BY-4.0). The split is in [docs/README.md](docs/README.md#license).
