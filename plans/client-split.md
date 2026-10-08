# Client split

`spook run` is the only server. It owns the player seat and the GM seat. The messages those seats speak move into `spookery-protocol`. A person sits through `spookery-cli`. The model sits through `spookery-ai`.

This change removes the `--mcp` catalog and the `spook gm --stdio` byte copy. It renames `spook host` to `spook ai-gm` and moves that program from `host/` to `ai/`. The play rules already written for that program stay with it.

This change does not specify what a person does at `spook gm`. That command is the name of a later client in `spookery-cli`. `spook ai-play` is a later subcommand in `spookery-ai`.

## Crates

Six crates. The kernel and tick do not depend on the protocol, the CLI, or the AI crate.

| Crate | Path | May link | Must not link |
|---|---|---|---|
| `spookery-protocol` | `protocol/` | `serde_json` | `tokio`, `rmcp`, `reqwest`, the kernel, the CLI, the AI crate |
| `spookery-kernel` | `src/kernel` | unchanged | the protocol, the CLI, the AI crate, `tokio`, `rmcp`, any model SDK |
| `spookery-tick` | `src/tick` | unchanged | the protocol, the CLI, the AI crate |
| `spookery` | `src/` | kernel, tick, protocol, CLI, AI, `tokio`, `rmcp` | `reqwest` by name. Seat code that calls into the CLI or the AI crate |
| `spookery-cli` | `cli/` | protocol, kernel | the AI crate, `reqwest`, `rmcp`, `tokio` |
| `spookery-ai` | `ai/` | protocol, kernel, an HTTP client, a runtime | tick, the CLI, `rmcp`, any model SDK |

`reqwest` stays in `ai/`. That tree is outside `src/`, so the `src/` byte scan still does not see it. `src/main.rs` dispatches `play`, `gm`, and `ai-gm` and does not name the HTTP client.

The word "host" on the operator digest stays. `spook status` still prints that digest. `DigestViewer::Host` is not this program.

## Protocol

`spookery-protocol` is the player messages and the GM messages, and the code that reads and writes them. Read and write turns a message into the line and a line back into the message. The two sets stay separate types. A player message has no GM field. Faces stay off the player socket.

The server, `spook play`, and `spook ai-gm` write those lines on their sockets. The MCP library stays in the server, which is the side that answers `tools/list` and `tools/call`.

The crate does not decide a turn, call a model, open a shard, apply an effect, or write the save.

## Commands

| Command | Crate | This change |
|---|---|---|
| `spook run` | server | The listen socket. Both seats. |
| `spook play` | `spookery-cli` | The current player client, moved. It still reads the start PC through `load_canon`. |
| `spook ai-gm` | `spookery-ai` | The current `spook host` program, renamed. Same seat, same turn rules. |
| `spook tick`, `migrate`, `status` | server | Stay. |
| `spook gm` | `spookery-cli`, later | Not built here. Until a later plan specifies it, the command is `NOT_IN_SPEC`. |
| `spook gm --stdio` | | Removed. |
| `spook run --mcp` | | Removed. The seat's `tools/list` is the menu. |

`spook ai-gm` still dials `127.0.0.1:9737`, still speaks `gm` and then the tool calls, and still does not call `open_shard`, `apply_effect`, or `write_commit`. One seat remains one seat.

## What leaves

The three profile name lists and the list-only stdio server in `src/mcp` are removed. `src/mcp` is removed with them.

Tick-profile perceive stays a separate function from sitting perceive. A player viewer, a missing `as`, or `as=gm` is still `UNSCOPED_PERCEIVE`. The function moves into the server crate. The proofs that call it keep calling it. It does not move into the kernel or the protocol crate.

`npc_act` still returns `LLM_DISABLED`.

Check notices stay on the GM socket and on the `commit_turn` result. The player wire stays `{"kind":"check","success":true|false}`.

## Spec sentences this replaces

In `docs/spec.md`:

- "Five crates." and the crate table, including the `spookery-mcp` and `spookery-host` rows, and "The bin, `spookery-mcp`, and `spookery-host` may link a runtime." The table in this plan adds the protocol, CLI, and AI edges to the crate table at `docs/spec.md:46`. Every allow and ban that table does not list stays, including `socket2`, `serde_json`, and the ban on a second tick implementation. `unchanged` means the current may-link cell.
- "Tokio in `spookery-mcp` is the stdio runtime for `--mcp`, which is list-only." That runtime leaves with the catalog.
- "Pack profiles stay three static lists" through the sentence that names `TABLE_TOOLS`, `TICK_TOOLS`, and `STUDIO_TOOLS`. The seat's `tools/list` is the menu.
- "`spook gm --stdio` remains the bridge. `spook gm` without `--stdio` is `NOT_IN_SPEC`." The bridge is removed. `spook gm` stays `NOT_IN_SPEC` until a later plan.
- "`--mcp` is list-only. It does not call `clear_crashed`, and it does not sit a GM." The flag is removed.
- The Host section's command name `spook host`, and the sentence that `spook gm <pack> --stdio` stays the human bridge. The program is `spook ai-gm`. The turn rules in that section stay.
- "The same roll notices are still sent as `check` events for the stdio bridge." They are sent on the GM socket.
- "`--as`, and `--mcp` combined with `--bind`, are `NOT_IN_SPEC` before the pack opens." The `--mcp` clause leaves with the flag. `--as` stays `NOT_IN_SPEC`.
- "The studio list-only server does not write `state/`." That server is removed with `src/mcp`.
- "`spook host` sends table text off the machine" in the privacy paragraph. The command in that sentence is `spook ai-gm`. The rest of the sentence stays.

`docs/product_spec.md` lists `src/mcp` and `/host` and says five crates.

## Sequence

Each step leaves the workspace building.

1. Add `protocol/` with the two message sets and their read and write. No caller switches yet. The crate's own tests round-trip a player line and a GM tool call.
2. Point the seat at those types for the player lines and the GM tool arguments and results. `cargo test --test promises` passes unchanged.
3. Move `host/` to `ai/`. The command is `ai-gm`. The `host` command is gone. The host feature passes against `spook ai-gm`.
4. Move `spook play` into `cli/`. The play paths in the promises pass.
5. Remove `spook gm --stdio`, `spook run --mcp`, and `src/mcp`. Move tick-profile perceive into the server. Delete `tests/features/profiles/three_menus.feature`. The `npc_act` promise still passes. The tick-perceive proof fails unless a player viewer, a missing `as`, and `as=gm` each return `UNSCOPED_PERCEIVE`. Point `manifests_do_not_name_a_model_crate` at `Cargo.toml`, `src/kernel/Cargo.toml`, and `src/tick/Cargo.toml`. Update the spec sentences named above.
