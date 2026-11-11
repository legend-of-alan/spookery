# Seat protocol

This page is the wire a player client or a GM speaks to `spook run`. [Engine guarantees](engine-guarantees.md) says what the engine promises. Three things make up the wire, and CI checks each one against the build:

- this page, whose examples are one live sitting of `packs/oneshot-docks`. A test plays that sitting and fails when an example and the table disagree.
- the JSON Schema, [`client.json`](../crates/protocol/schema/client.json) for what a client sends and [`server.json`](../crates/protocol/schema/server.json) for what the table sends. A test fails when the schema `spookery-protocol` derives with its `schema` feature is not the published one.
- the recorded session, [`v2.jsonl`](../crates/protocol/sessions/v2.jsonl), a player and a GM recorded against version 2. The `compatibility-replay` CI job replays it against each build.

In the examples, Rowan is the player, in the shop, at game time 1080. A tick is game time. A round is one batch of queued moves.

## Connect

The game port is a WebSocket endpoint. A player opens `ws://127.0.0.1:<game>/player` and a GM opens `ws://127.0.0.1:<game>/gm`. The default game port is `127.0.0.1:9737`. The MCP endpoint, for a GM that speaks MCP, is streamable HTTP at `http://127.0.0.1:<mcp>/mcp`, on the game port plus one, so `9738` by default.

`--addr 127.0.0.1:0` asks the operating system for the game port, and then the MCP endpoint is a bind of its own. `--mcp-addr` sets the MCP address, port 0 included. Both binds are loopback. Any other host is refused before the pack opens. Once both are bound, and before the table serves, `spook run` prints one line on stdout with the two addresses:

<!-- example: listening -->
```json
{"kind":"listening","game":"127.0.0.1:58439","mcp":"127.0.0.1:58440"}
```

Both endpoints take `Host: 127.0.0.1` only, so `localhost` is 403. An `Origin`, when sent, is `http://127.0.0.1` on any port. Anything else is 403. There is no account and no connection secret.

## The envelope

Every message, the client's and the table's alike, is one JSON object in one WebSocket text message: `id`, `seq`, `reply_to` and a nested `body`. A binary message is `SCHEMA`.

- `id` is the client's choice. The table copies it into `reply_to` on the one reply to that message.
- `seq` is set by the table on everything it sends but the hello, from one counter for the whole table. A message several seats get has the same `seq` on each. A seat's `seq`s only grow, but they skip the messages other seats got.
- A client's `body` is a command `kind` and its `args`, an object that may be left out when the command takes none.
- A table's `body` is tagged by `kind`.

<!-- example: player-claim -->
```json
{"id":"c1","body":{"kind":"claim","args":{"actor":"pc_rowan"}}}
```

<!-- example: player-claimed -->
```json
{"seq":1,"reply_to":"c1","body":{"kind":"claimed","actor":"pc_rowan","token":"<token>"}}
```

## Hello

The table speaks first, with `version`, the `commands` this seat may send, and, on `/player`, `open`, the PCs nobody holds now, `taken`, the PCs held now, and `pc_names`, each of those PCs' display name by id. A claim on an open PC is still refused with `TABLE_FULL` while every player seat is held. Each is left out when empty. A player's commands are the table's own and then every verb the pack's rules name.

<!-- example: player-hello -->
```json
{"body":{"kind":"hello","version":2,"commands":["claim","release","say","look","help","map","cancel","drop","grit","take","travel","unwield","wait","wield"],"open":["pc_harbormouse","pc_rowan","pc_wren"],"pc_names":{"pc_harbormouse":"Harbormouse","pc_rowan":"Rowan","pc_wren":"Wren"}}}
```

<!-- example: gm-hello -->
```json
{"body":{"kind":"hello","version":2,"commands":["status","perceive","queue","say","commit","release"]}}
```

One GM sits at a time, on `/gm` or on `/mcp`. A second GM on `/gm` gets one `refused` with `TABLE_FULL`, and the connection closes.

## Commands

Each command reads `args` as one type, and the schema's `$defs` in `client.json` hold each type under the name below. A command a seat's hello did not list is `SCHEMA`, and so are arguments of the wrong shape or with a key the type does not name. Nothing is written for a refused command.

| Command | Seat | `args` | Reply |
|---|---|---|---|
| `claim` | player | `ClaimArgs`: `actor`, or `token` and `last_seq` | `claimed`, then a `resume` and a `view` |
| `release` | both | `NoArgs` | `released` |
| `say` | both | `SayArgs`: `address`, `text`, and `place` or `actor` | the sender's own `say` |
| `look` | player | `NoArgs` | `view` |
| `help` | player | `NoArgs` | `help` |
| `map` | player | `NoArgs` | `map` |
| `cancel` | player | `NoArgs` | `queued` |
| a pack verb | player | `MoveArgs`: `target`, when the verb takes one | `queued` |
| `status` | GM | `NoArgs` | `status` |
| `perceive` | GM | `PerceiveArgs`: `as`, `max_body_chars` | `view` |
| `queue` | GM | `IntentArgs`: `actor_id`, `move`, `id`, `target`, `to`, `round_index`, `rationale_code` | `queued` |
| `commit` | GM | `CommitArgs`: `include_round`, `session_id` | a `round` for each round played, then `committed` |

### claim and release

A player claims one PC, and the table holds three claims at most. A command other than `claim` before a claim is `CHOOSE_PC`. The hello already listed the open PCs:

<!-- example: player-choose-pc -->
```json
{"seq":5,"reply_to":"c1","body":{"kind":"refused","code":"CHOOSE_PC"}}
```

`actor` names the PC by id or by display name, ignoring case and outer spaces. A PC another claim holds is `SEAT_TAKEN`, a fourth claim is `TABLE_FULL`, a name two PCs share is `AMBIGUOUS_NAME`, and anything that names no live PC is `SCHEMA`. A refused claim leaves the connection up.

Only `release` ends a claim. A connection that closes or is dropped leaves its PC held and present, so no other player can claim it, the GM cannot queue for it, and rounds still count it. The claim lasts until a `release` or until `spook run` stops. Presence is kept in memory and does not bump `state_rev`. The GM hears a `presence` whenever a claim starts or ends:

<!-- example: gm-presence -->
```json
{"seq":4,"body":{"kind":"presence","actors_present":["pc_rowan"]}}
```

`token` in `claimed` takes the PC back from a new connection, so treat it as a secret. A held claim keeps the newest 256 messages sent to it. `claim` with `token` and `last_seq`, the last `seq` the old connection read, answers with `missed`, the kept messages after `last_seq`, oldest first. A missing `last_seq` means 0. `gap` is `true` when a message after `last_seq` was dropped. The `resume` and `view` after it are current either way. A connection that held the claim before lets go of it and stays up. A released claim's token is `UNKNOWN_ID`.

<!-- example: player-claim-token -->
```json
{"id":"c1","body":{"kind":"claim","args":{"token":"<token>","last_seq":21}}}
```

<!-- example: player-claimed-token -->
```json
{"seq":23,"reply_to":"c1","body":{"kind":"claimed","actor":"pc_rowan","token":"<token>","missed":[{"seq":22,"body":{"kind":"say","from":"gm","address":"table","text":"still there?"}}]}}
```

A new claim's `claimed` carries `scrollback` instead, the says this PC heard before, oldest first.

A GM's `release` frees the seat before its reply. A later command on that connection is `NOT_IN_SPEC`. With the GM seat empty, each player move commits its own round.

### say

`address` is `table`, `place` with a `place` id, or `actor` with an `actor` id. `text` is at most 4096 characters. The table sets `from` from the connection: `player:<id>` or `gm`. A `from` in `args` is `SCHEMA`. A say goes on the session tape and takes no round. Every seat that hears it gets one `say` under one `seq`, and the sender's copy is the reply. The GM's copy lists `recipients`, the seated players who heard it.

<!-- example: player-say -->
```json
{"id":"c2","body":{"kind":"say","args":{"address":"table","text":"evening"}}}
```

<!-- example: player-said -->
```json
{"seq":6,"reply_to":"c2","body":{"kind":"say","from":"player:pc_rowan","address":"table","text":"evening"}}
```

<!-- example: gm-heard -->
```json
{"seq":6,"body":{"kind":"say","from":"player:pc_rowan","address":"table","recipients":["pc_rowan"],"text":"evening"}}
```

### Moves

A move is a pack verb, its target in `target`. The target is an id or a display name the player sees, ignoring case and outer spaces, and an exit also answers to the place it goes to, by id or name. A name that two things in sight share is refused `AMBIGUOUS_NAME` before it is queued, with that name in `detail`. With a GM seated, the move waits for `commit`. The reply is `queued`, and the GM gets the same `queued` under the same `seq`. `intent` is the queue id. With no GM seated, the move commits its own round, and the reply comes after that round is written.

<!-- example: player-grit -->
```json
{"id":"c3","body":{"kind":"grit"}}
```

<!-- example: player-queued -->
```json
{"seq":9,"reply_to":"c3","body":{"kind":"queued","intent":"ed0db120f54c1447842c340c5d523932d1518c796c886f24b1030a9b4a12f73a","actor":"pc_rowan","move":"grit"}}
```

`cancel` is a move too, and a busy player may send it. It drops the PC's own action under way when its rule says `cancellable`, with no refund, and the action never finishes. A walk is never cancelled. With nothing to cancel, or an action whose rule does not allow it, `cancel` is refused `INELIGIBLE` and the action still lands.

A move the table can judge at once is refused before it is queued. Here Rowan sent `travel` with `target` `stairs`, and `detail` is the pack's locked text:

<!-- example: player-no-exit -->
```json
{"seq":21,"reply_to":"c5","body":{"kind":"refused","code":"NO_EXIT","detail":"The stair door is locked."}}
```

`look` takes no round and answers with a fresh `view`. `help` and `map` take none either, and answer with data the client prints. `help` lists the player commands but `claim`, then each pack verb this PC can use here and now. Each command has a `target`: `none`, `actor`, `place`, `exit` or `item`. `names` lists what the PC can name for it now, by the names the player reads: the exits from here, the actors or items it may name, or the places it visited. A name two things in sight share carries the id after it, as `Guard (npc_guard_east)`. A cast member's or item's verb carries that `owner` id, and its name as its one name, so it reads as `ask Briar`, and it shows only while they are in sight. A verb whose `if` fails now is left out, but cost is not checked. The exits and the busy line are in the `view`, not in `help`.

<!-- example: player-help -->
```json
{"seq":30,"reply_to":"c3","body":{"kind":"help","commands":[
  {"name":"release","target":"none"},
  {"name":"say","target":"none"},
  {"name":"look","target":"none"},
  {"name":"help","target":"none"},
  {"name":"map","target":"none"},
  {"name":"cancel","target":"none"},
  {"name":"drop","target":"item","names":["Lamp oil"]},
  {"name":"grit","target":"none"},
  {"name":"take","target":"item","names":["Lamp oil"]},
  {"name":"travel","target":"exit","names":["Docks Street","stairs"]},
  {"name":"unwield","target":"none"},
  {"name":"wait","target":"none"},
  {"name":"wield","target":"item","names":["Lamp oil"]}
]}}
```

`map` lists each place the PC's save says they visited, by display name, with the exits they saw there when they last stood there at the end of a round. A hidden exit revealed since then shows only after they go back. Where each exit goes, its ticks and `locked` are read now.

<!-- example: player-map -->
```json
{"seq":31,"reply_to":"c4","body":{"kind":"map","places":[{"name":"Docks Shop","exits":[{"name":"docks_street","to":"docks_street","place_name":"Docks Street","ticks":0,"locked":false},{"name":"stairs","to":"mira_rooms","place_name":"Mira's Rooms","ticks":2,"locked":true}]}]}}
```

### status

The table's state for a GM. `world_time` is the game clock in ticks. `pack_rev` is the canon hash. A client that opened the pack on disk should compare it and leave on a mismatch. `last_round_at` is the save's stamp. `queued` lists the waiting moves. `pack` is the move menu, the cast kinds, and the place and cast prose.

<!-- example: gm-status -->
```json
{
  "seq": 7,
  "reply_to": "c1",
  "body": {
    "kind": "status",
    "presence": {
      "players": 1,
      "actors_present": [
        "pc_rowan"
      ]
    },
    "world_time": 1080,
    "round_index": 0,
    "state_rev": 0,
    "pack_rev": "blake2s:d9007ad32e134c6bf6292316c00fe04a015504c3ea6d99455fd791986b995d82",
    "last_round_at": "2026-01-01T00:00:00Z",
    "queued": [],
    "pack": {
      "menu": [
        {
          "name": "leave_door_note",
          "kind": "habit",
          "target": "none"
        },
        {
          "name": "restock_lamp_oil",
          "kind": "habit",
          "target": "none"
        },
        {
          "name": "wait",
          "kind": "verb",
          "target": "none"
        },
        {
          "name": "travel",
          "kind": "verb",
          "target": "exit"
        },
        {
          "name": "grit",
          "kind": "verb",
          "target": "none"
        },
        {
          "name": "take",
          "kind": "verb",
          "target": "item"
        },
        {
          "name": "drop",
          "kind": "verb",
          "target": "item"
        },
        {
          "name": "wield",
          "kind": "verb",
          "target": "item"
        },
        {
          "name": "unwield",
          "kind": "verb",
          "target": "none"
        }
      ],
      "kinds": {
        "aldric": "npc",
        "bram": "npc",
        "mira": "npc",
        "pc_harbormouse": "pc_template",
        "pc_rowan": "pc_template",
        "pc_wren": "pc_template"
      },
      "places": {
        "docks_shop": "The shop faces the water. Tins sit in a row.\n",
        "docks_street": "The street runs between the shop and the rooms.\n",
        "mira_rooms": "A lamp burns past the shop's back stair.\n"
      },
      "cast": {
        "aldric": "Aldric sweeps the shop and does not keep the books.\n",
        "bram": "Bram walks the night watch between the street and the shop.\n",
        "mira": "Mira keeps the shop neat and the tin out of sight.\n",
        "pc_harbormouse": "Harbormouse walks the night street and sleeps in a coil of rope.\n",
        "pc_rowan": "Rowan keeps the shop and walks the street.\n",
        "pc_wren": "Wren mends nets on the street and keeps the tide chart in their head.\n"
      }
    }
  }
}
```

### perceive

`as` is a bare actor id, not `player:<id>` or `npc:<id>`. A missing `as`, or `gm`, is `UNSCOPED_PERCEIVE`. `max_body_chars` is the prose budget, 800 by default. The reply is the `view` that actor would get.

### queue

`actor_id` and `move` are required. `id` becomes the queue id, and the table mints one when it is left out. A PC a player holds takes only that player's moves, so the GM's move for it is `ACTOR_MISMATCH`.

<!-- example: gm-queue -->
```json
{"id":"c2","body":{"kind":"queue","args":{"actor_id":"aldric","move":"wait","id":"gm-wait"}}}
```

<!-- example: gm-queued -->
```json
{"seq":8,"reply_to":"c2","body":{"kind":"queued","intent":"gm-wait","actor":"aldric","move":"wait"}}
```

### commit

`include_round: true` plays the queue in queue order as one round. `include_round: false` while a move is queued is `NOT_IN_SPEC`, and the queue stays. An empty queue writes no round, unless every seated PC is busy, and then the table fast-forwards. Every seat gets one `round` for each round played, and the GM's reply comes after them.

<!-- example: gm-commit -->
```json
{"id":"c3","body":{"kind":"commit","args":{"include_round":true}}}
```

<!-- example: gm-round -->
```json
{"seq":10,"body":{"kind":"round","round_index":1,"world_time":1085}}
```

<!-- example: gm-committed -->
```json
{"seq":14,"reply_to":"c3","body":{"kind":"committed","state_rev":1,"changed_keys":["last_act_round aldric","last_act_round mira","last_act_round pc_rowan","live_acts_this_window aldric","live_acts_this_window mira","live_acts_this_window pc_rowan","note note_mira_door"],"journal_path":"state/journal/bundle-0000.jsonl","rolls":[{"faces":[5,2],"stat":1,"total":8,"target":0,"outcome":"success"}]}}
```

`changed_keys` are the state keys the round changed, every one of them: a meter is `actor_meter <actor> <id>` or `world_meter <id>`, a modifier is `modifier <actor> <id>`, and a flag is `actor_flag <actor> <key>` or `world_flag <key>`. `journal_path` is the round's journal file, or null when the round wrote none. `rolls` are the checks rolled, faces and all. Faces go to the GM only.

## Bodies

A client ignores a `kind` or a field it does not know. `server.json` gives every field.

| Kind | Who gets it | What it says |
|---|---|---|
| `hello` | the new connection | `version`, `commands`, `open`, `taken` |
| `claimed` | the claiming player | `actor`, `token`, and `scrollback` or `missed` and `gap` |
| `released` | the releasing seat | the claim or the GM seat is free |
| `presence` | the GM | `actors_present` after a claim starts or ends |
| `refused` | the sender | `code` and, when the refusal names something, `detail` |
| `queued` | the sender and the GM | `intent`, `actor`, `move`, `target` |
| `say` | every seat that hears it | `from`, `address`, `place` or `actor`, `text`, and for the GM `recipients` |
| `round` | every seat | `round_index` and `world_time` of one committed round |
| `check` | the player who rolled | `outcome`: `success`, `partial` or `miss` |
| `committed` | the GM | the reply to `commit` |
| `status` | the GM | the reply to `status` |
| `view` | a player, or the GM on `perceive` | what one actor can see |
| `resume` | a player | the `notes` addressed to that PC |
| `lines` | a player | text to print: one narrate line, or the changes that player saw |
| `help` | the asking player | `commands`, each with `name`, `target`, `names` and `owner` |
| `map` | the asking player | `places`, each with `name` and `exits` |

### view and resume

`viewer` is the bare actor id. `place_name`, `actor_names` and each item's `name` are the pack's display names, and `body` is the place prose. `known_facts` hold only the facts that actor knows, with the prose that actor may read. `meters` are the viewer's own, without hidden ones. `world_meters` are the world's meters whose pack row says `hidden: false`, at their current values, so a player who comes back sees them. `exits` is left out when the place has none. `busy` says what keeps the viewer busy and until when, and is left out when they are free: `on the stairs until tick 1300` on a walk, `chanting until tick 1300` on any other timed move, read from its verb, and `recovering until tick 1300` in a cooldown. A busy player may still send a move, and it is refused `BUSY`, except `cancel` (see [Moves](#moves)). `cant_act` says why the viewer can't send a move at all, and is left out when they can, so `View::can_act()` is `cant_act` being empty. Nothing sets it yet.

<!-- example: player-resume -->
```json
{"seq":2,"body":{"kind":"resume"}}
```

<!-- example: player-view -->
```json
{
  "seq": 3,
  "body": {
    "kind": "view",
    "viewer": "pc_rowan",
    "place": "docks_shop",
    "place_name": "Docks Shop",
    "body": "The shop faces the water. Tins sit in a row.",
    "actor_names": [
      "Aldric",
      "Rowan"
    ],
    "visible_actors": [
      "aldric",
      "pc_rowan"
    ],
    "known_facts": [
      {
        "id": "fact_oil_on_shelf",
        "text": "Oil is on the shelf.",
        "owner_body": "The shop faces the water. Tins sit in a row.\n"
      }
    ],
    "notes": [],
    "unknown_mentions": [],
    "items": [
      {
        "id": "lamp_oil",
        "count": 4,
        "name": "Lamp oil"
      }
    ],
    "meters": [
      {
        "id": "assassination_debt",
        "value": 2,
        "max": 6
      },
      {
        "id": "grit",
        "value": 1,
        "max": 110
      },
      {
        "id": "rest",
        "value": 3,
        "max": 5
      }
    ],
    "habits": [],
    "exits": [
      {
        "name": "docks_street",
        "to": "docks_street",
        "place_name": "Docks Street",
        "ticks": 0,
        "locked": false
      },
      {
        "name": "stairs",
        "to": "mira_rooms",
        "place_name": "Mira's Rooms",
        "ticks": 2,
        "locked": true
      }
    ]
  }
}
```

### After a round

Each seated player gets, in order: the `round`, a `check` for each of their own rolls, a `refused` for each of their own moves the round refused, a fresh `resume` and `view`, a `lines` for each narrate line they may hear, and a last `lines` with the meter changes and items gained or lost that player may know. A player is told their own meter changes and the world's shown ones, not another actor's, even in the same place, and never a flag change. A `lines` that would be empty is not sent, and a narrate line that would show a fact the player may not know is left out. An NPC's roll or refused habit sends no player message.

<!-- example: player-round -->
```json
{"seq":10,"body":{"kind":"round","round_index":1,"world_time":1085}}
```

<!-- example: player-check -->
```json
{"seq":11,"body":{"kind":"check","outcome":"success"}}
```

<!-- example: player-lines -->
```json
{"seq":19,"body":{"kind":"lines","text":"You picked up the Lamp oil."}}
```

## MCP

`/mcp` is a translator onto the GM commands. `tools/list` has one tool per GM command, named after it, in hello order, and each tool's input schema is its command's arguments. A `tools/call` is sent to the table as that command, the arguments as its `args` and the JSON-RPC id as its `id`. A tool that is not a GM command is `SCHEMA`.

<!-- example: mcp-call -->
```json
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"queue","arguments":{"actor_id":"aldric","move":"wait","id":"mcp-wait"}}}
```

The result's `structuredContent` is `messages`: every message the command sent this seat, in order, the reply last.

<!-- example: mcp-result -->
```json
{"messages":[{"seq":27,"reply_to":"1","body":{"kind":"queued","intent":"mcp-wait","actor":"aldric","move":"wait"}}]}
```

Every other message the table sends this seat, the hello first, comes as a `spook/event` notification whose params are the message:

<!-- example: mcp-event -->
```json
{"jsonrpc":"2.0","method":"spook/event","params":{"seq":28,"body":{"kind":"queued","intent":"f756bf6b7077bf5c623ac2d933339f8357220b557f99dbf3492e145ff42e16a8","actor":"pc_rowan","move":"grit"}}}
```

An `initialize` opens a session and is never refused for a full table. POST it with `Accept` listing both `application/json` and `text/event-stream` and with `Content-Type: application/json`. Keep the `mcp-session-id` response header, and send the negotiated `protocolVersion` as `MCP-Protocol-Version` on every later request.

The session's `GET /mcp` event stream takes the GM seat, and its first event is the hello. Call a command only after the hello: a session with no event stream holds no seat, so its tool calls are `NOT_IN_SPEC`. Hold that one stream open: the seat is freed when it closes, when a DELETE names the session, or on `release`. A GET while the seat is held, or a second GET for a session that already has its stream, is refused on that stream as its only event, and nothing else closes:

<!-- example: mcp-table-full -->
```json
{"jsonrpc":"2.0","method":"spook/event","params":{"seq":29,"body":{"kind":"refused","code":"TABLE_FULL"}}}
```

## Limits

- A client message is at most 8 KiB. A longer one is `LINE_TOO_LONG` and the connection stays up. One over 64 KiB closes the connection with no reply. An MCP request body is at most 1 MiB, and a longer one is HTTP 413.
- A connection that sends nothing for 10 seconds after the hello is closed.
- A player connection may send 5 messages a second, bursting to 10. A message past that waits on its own connection, and other seats are not slowed.
- Each connection queues at most 256 messages and 64 KiB for sending. A client that stops reading is dropped like any closed connection, and other seats keep their messages.

## Errors

A refusal is a `refused` body, and the connection stays up. `detail` is there only when the refusal names a thing: the locked text or the unaffordable id of a move.

| Code | What the client should do |
|---|---|
| `TABLE_FULL` | Stop. Three PCs are claimed, or the GM seat is held. Try again only after a seat is free. |
| `SEAT_TAKEN` | That PC is held. Claim another. |
| `CHOOSE_PC` | Claim one of the PCs in the hello's `open`. An empty `open` means none is free. |
| `SCHEMA` | Fix the message: its shape, its command, or its arguments. |
| `NOT_IN_SPEC` | The verb has no rule that applies, `commit` set `include_round` false with a move queued, or the seat was released or, over MCP, never taken. |
| `LINE_TOO_LONG` | Send a message under 8 KiB. |
| `UNSCOPED_PERCEIVE` | Send `perceive` with `as` set to a bare actor id. |
| `ACTOR_MISMATCH` | That actor is a player's, or the move is not this actor's. |
| `NO_EXIT`, `CANT_AFFORD`, `MISSING_ITEM`, `NOT_A_WEAPON`, `BUSY` | The move was not queued. `BUSY` clears when the character is free. |
| `AMBIGUOUS_NAME` | Two things in sight, or two PCs, have the name `detail` gives. Name one by the id `help` shows after it. |
| `UNKNOWN_MOVE`, `UNKNOWN_ID` | That move or id is not in the world, or the token is no claim's. |
| `INELIGIBLE`, `OUT_OF_RANGE`, `BUDGET`, `UNKNOWN_EFFECT`, `ILLEGAL_KNOWLEDGE`, `VIEWER_MISMATCH`, `ENGINE_ON_PLAYER`, `TABLE_EMPTY` | The rules refused the change, and nothing of it was kept. |
| `COMMIT_CONFLICT` | Another process holds the pack. Stop. |
| `HAND_EDIT` | The save was edited by hand. Stop until it is migrated. |
| `JOURNAL_DIVERGED` | The journal no longer replays. `spook verify` names the record. Stop. |
| `STORE_IO` | The disk refused a read or a write. Stop until the operator fixes it. |
| `PACK_REV_MISMATCH` | The save does not match the pack. Stop. |
| `RULES_REV_MISMATCH` | The save was written by other rules. Start a fresh save. This fails at pack open, before any seat. |

The MCP endpoint answers with an HTTP status before any seat: 400 for an `MCP-Protocol-Version` it does not know, 403 for a wrong `Host` or `Origin`, 405 for a DELETE that names a version with no session, 406 for an `Accept` without both types, and 413 for a body over 1 MiB.

## Compatibility

- A client ignores a body `kind` and a field it does not know.
- Changes only add. A field never changes meaning, and a new meaning gets a new name.
- The hello announces `version` and the commands. A command is removed only after one release of being marked as going away. Version 2 has no such command and no marker yet. The marker is added to the hello before the first command needs it.
- Version 2 broke version 1 once, on purpose: clocks left the wire with world clocks. `clocks` left `view` and `resume`, `pause` left `status`, `PAUSED_CLOCK` left the refusal codes, and `view.status` became `view.cant_act`. `view.world_meters` was added. The session recorded against version 1 was replaced by one recorded against version 2.
- Version 2 dropped one field before any release, on purpose: `CHOOSE_PC` no longer lists the open PCs in `detail`, since the hello's `open` already does, so `detail` is always a string. The version stayed 2 because no build had shipped.
- Version 2 renamed two things before any release, on purpose, when game time became ticks: an exit's `minutes` became `ticks`, and the changed key `live_acts_this_hour` became `live_acts_this_window`. The recording took the docks pack's new `pack_rev` with them. The version stayed 2 because no build had shipped.
- Version 2 renamed one more thing before any release, on purpose, when the engine stopped counting days: the changed key `idle_acts_today` became `idle_acts_this_window`, and `cant_act` names where a journey ends as `until tick N`, not a clock time. The recording took the docks pack's new `pack_rev` with it. The version stayed 2 because no build had shipped.
- Version 2 narrowed one field before any release, on purpose, when every move got one start and finish: `cant_act` now means the viewer can't send a move at all, and being busy moved to the new `busy`. A walk's line moved with it. The version stayed 2 because no build had shipped.
- Version 2 narrowed `status` before any release, on purpose: `pack` is now required, so a GM client no longer allows for a server that leaves it out. The version stayed 2 because no build had shipped.
- Version 2 renamed the command fields before any release, on purpose, so a client body is named by `kind` like a server body: a client body's `word` became `kind`, and the `words` of `hello` and `help` became `commands`. The command names did not change. The recording took the new names with it. The version stayed 2 because no build had shipped.
- The recorded session must still replay. Each message it got must still arrive at that seat, in order, with every field it had and the same value. New kinds and fields may come between. Only `seq`, which a new kind moves, and `token`, which each table mints, may differ.
- The journal and refusal codes stay strict: `spook verify` fails on a code it does not know.
