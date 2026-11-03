# Seat protocol

This page is the seat protocol for a player client or a GM integration. `spook run` is the only server. It owns both seats. [Engine guarantees](engine-guarantees.md) is what the engine promises. This page is how to speak to it.

The examples are one live sitting of `packs/oneshot-docks`. Rowan is the player, in the shop, at game time 1080. A test plays that sitting and fails when a fenced example and the table disagree. The `id` on a tool call or a client message is the caller's. The ports on the listening line are the ports that run chose.

A tick is game time. A round is one batch of queued moves.

## Connect

The game port is a WebSocket endpoint. A player opens `ws://127.0.0.1:<game>/player` and a GM opens `ws://127.0.0.1:<game>/gm`. The default is `127.0.0.1:9737`. It takes the same `Host` and `Origin` rules as the MCP endpoint below.

The MCP endpoint is streamable HTTP at `http://127.0.0.1:<port>/mcp`. When the game port is fixed, the MCP endpoint's port is the game port plus one, so the default MCP endpoint is `http://127.0.0.1:9738/mcp`. `--addr 127.0.0.1:0` asks the operating system for the game port, and the MCP endpoint is its own bind. `--mcp-addr 127.0.0.1:<port>` overrides the MCP endpoint's address, including port 0. Do not add one to a port-0 game port. Read the listening line.

Both binds are loopback. Any other host is refused before the pack opens.

After both sockets are bound, and before the table serves, `spook run` prints one line on stdout:

<!-- example: listening -->
```json
{
  "kind": "listening",
  "game": "127.0.0.1:61762",
  "mcp": "127.0.0.1:61763"
}
```

`game` and `mcp` are `127.0.0.1` and a port. They are different ports. This run happened to draw two adjacent ports. The next run draws its own.

The MCP endpoint requires:

- `Host: 127.0.0.1`. `localhost` is 403.
- `Origin`, when the request sends one, is `http://127.0.0.1` on any port. Anything else is 403. A missing `Origin` is accepted.
- `Accept` lists both `application/json` and `text/event-stream`.
- `Content-Type: application/json` on a POST.
- A body of at most 1 MiB. A longer body is 413 and does not take the seat.
- `MCP-Protocol-Version`, when present, names a version the server knows. An unknown version is 400. A missing header is accepted.

There is no account and no connection secret.

## Player seat

Open a WebSocket to `ws://127.0.0.1:<game>/player`. Each text message is one JSON object. A binary message is `SCHEMA`.

### The envelope

Every message is one envelope, the client's and the table's alike: `id`, `seq`, `reply_to`, and a nested `body`. The client picks `id`. The table copies it into `reply_to` on the one reply to that message. `seq` is set by the table, from one counter for the whole table, on every message it sends but the hello. A message seen by several seats carries the same `seq` on each. The body never shares a field with the envelope.

A client message's body is a `word` and its `args`. `args` is an object, and may be left out when the word takes none. A table message's body is tagged by `kind`. A client ignores a `kind` or a field it does not know.

### Hello

The table speaks first. The hello lists the version, the words this seat may send, and `open`, the PCs a claim may take now. The words are the table's own and then every verb the pack's rules name. `spook play` prompts for a PC when more than one is open.

<!-- example: player-hello -->
```json
{
  "body": {
    "kind": "hello",
    "version": 1,
    "words": [
      "claim",
      "release",
      "say",
      "look",
      "drop",
      "grit",
      "take",
      "travel",
      "unwield",
      "wait",
      "wield"
    ],
    "open": [
      "pc_harbormouse",
      "pc_rowan",
      "pc_wren"
    ]
  }
}
```

`claim` takes one PC. The reply is `claimed`, with `token`, which takes this PC back after a dropped connection, and `scrollback`, the earlier says this PC heard, oldest first. A `resume` and a `view` follow. Treat the token as a secret: anyone who sends it holds the PC.

<!-- example: player-claim -->
```json
{
  "id": "c1",
  "body": {
    "word": "claim",
    "args": {
      "actor": "pc_rowan"
    }
  }
}
```

<!-- example: player-claimed -->
```json
{
  "seq": 1,
  "reply_to": "c1",
  "body": {
    "kind": "claimed",
    "actor": "pc_rowan",
    "token": "<token>"
  }
}
```

<!-- example: player-resume -->
```json
{
  "seq": 2,
  "body": {
    "kind": "resume",
    "clocks": [
      {
        "id": "assassination_debt",
        "value": 2,
        "max": 6,
        "paused": true
      }
    ]
  }
}
```

<!-- example: player-perceive -->
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
    "clocks": [
      {
        "id": "assassination_debt",
        "value": 2,
        "max": 6,
        "paused": true
      }
    ],
    "items": [
      {
        "id": "lamp_oil",
        "count": 4
      }
    ],
    "meters": [
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
        "minutes": 0,
        "locked": false
      },
      {
        "name": "stairs",
        "to": "mira_rooms",
        "place_name": "Mira's Rooms",
        "minutes": 2,
        "locked": true
      }
    ]
  }
}
```

`resume` carries the notes addressed to that viewer (`notes`) and their own clocks (`clocks`). `view` is what that viewer can see. `viewer` is the bare actor id, the same form `as` takes. It is not `player:<id>` or `npc:<id>`. `place_name` and `actor_names` are the pack's display names. `body` is the place body. `meters` is that viewer's own meters. A hidden meter is left out. `value` is the current number and `max` is the cap. `exits` is omitted when the place has none. `status` is omitted unless the viewer is still travelling, and then it is `on the stairs until 21:40`. Fact bodies are the place prose the viewer is allowed to read.

A claim keeps presence in memory and does not bump `state_rev`. The cap is three claims, in the host, not in the pack. `release` gives the PC back. The connection stays up and may claim again. Only `release` ends a claim. A connection that closes or is dropped leaves its PC held and present, so no other player can claim it, the GM cannot queue for it, and rounds still count it. The idle clock waits while any claim is held. There is no timer: the claim lasts until a `release` or until `spook run` stops.

The held claim keeps the newest 256 messages sent to it, whether or not a connection reads them. That is the one catch-up window. A new connection takes the PC back with `claim` and `token`, and `after`, the last `seq` it read. The `claimed` reply's `missed` lists the kept messages after `after`, oldest first, each as it was sent. `gap` is `true` when a message after `after` has fallen out of the window. The `resume` and `view` that follow are current either way. A connection that still held the claim lets go of it and stays up unclaimed. A token no claim holds, after a `release` for one, is `UNKNOWN_ID`.

<!-- example: player-claim-token -->
```json
{
  "id": "c1",
  "body": {
    "word": "claim",
    "args": {
      "token": "<token>",
      "after": 23
    }
  }
}
```

<!-- example: player-claimed-token -->
```json
{
  "seq": 35,
  "reply_to": "c1",
  "body": {
    "kind": "claimed",
    "actor": "pc_rowan",
    "token": "<token>",
    "missed": [
      {
        "seq": 34,
        "body": {
          "kind": "say",
          "from": "gm",
          "address": "table",
          "text": "still there?"
        }
      }
    ]
  }
}
```

A refused claim leaves the connection up. The PC is already held:

<!-- example: player-seat-taken -->
```json
{
  "seq": 5,
  "reply_to": "c1",
  "body": {
    "kind": "refused",
    "code": "SEAT_TAKEN"
  }
}
```

A fourth claim, after three are held:

<!-- example: player-session-full -->
```json
{
  "seq": 32,
  "reply_to": "c1",
  "body": {
    "kind": "refused",
    "code": "TABLE_FULL"
  }
}
```

An id that is not a live PC:

<!-- example: player-unknown-pc -->
```json
{
  "seq": 6,
  "reply_to": "c1",
  "body": {
    "kind": "refused",
    "code": "SCHEMA"
  }
}
```

Any other word before a claim, after Rowan is sat. `detail` is the open PCs:

<!-- example: player-choose-pc -->
```json
{
  "seq": 7,
  "reply_to": "c1",
  "body": {
    "kind": "refused",
    "code": "CHOOSE_PC",
    "detail": [
      "pc_harbormouse",
      "pc_wren"
    ]
  }
}
```

A message over 8 KiB is `LINE_TOO_LONG`, and the connection stays up. One over 64 KiB closes the connection with no reply. A connection that sends nothing for 10 seconds after the hello closes, and nothing is committed.

<!-- example: player-line-too-long -->
```json
{
  "seq": 8,
  "body": {
    "kind": "refused",
    "code": "LINE_TOO_LONG"
  }
}
```

### Words the player sends

`look` does not take a round. The reply is a fresh `view`, the same shape a claim and a round send. The table sends no text for it. `spook play` prints the view, and writes `help` and `map` itself without sending anything: `help` from the hello's words and the last view's exits, and `map` from the views it was shown since it started.

<!-- example: player-look -->
```json
{
  "id": "c2",
  "body": {
    "word": "look"
  }
}
```

<!-- example: player-look-view -->
```json
{
  "seq": 4,
  "reply_to": "c2",
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
    "clocks": [
      {
        "id": "assassination_debt",
        "value": 2,
        "max": 6,
        "paused": true
      }
    ],
    "items": [
      {
        "id": "lamp_oil",
        "count": 4
      }
    ],
    "meters": [
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
        "minutes": 0,
        "locked": false
      },
      {
        "name": "stairs",
        "to": "mira_rooms",
        "place_name": "Mira's Rooms",
        "minutes": 2,
        "locked": true
      }
    ]
  }
}
```

`say` is speech. It appends the tape and does not take a round. Its args are the GM's `say` arguments: `address` is `table`, `place` with the speaker's own `place`, or `actor` with an `actor` id. The table sets `from` from the connection.

<!-- example: player-say -->
```json
{
  "id": "c3",
  "body": {
    "word": "say",
    "args": {
      "address": "table",
      "text": "evening"
    }
  }
}
```

Every seat that hears it gets one `say` with one `seq`. The speaker's copy is the reply:

<!-- example: player-say-frame -->
```json
{
  "seq": 9,
  "reply_to": "c3",
  "body": {
    "kind": "say",
    "from": "player:pc_rowan",
    "address": "table",
    "text": "evening"
  }
}
```

A claim's `scrollback` holds the earlier says. The terminal prints them under one `Earlier:` heading and leaves a blank line before the next live text.

<!-- example: player-replay-say -->
```json
{
  "seq": 40,
  "reply_to": "c1",
  "body": {
    "kind": "claimed",
    "actor": "pc_rowan",
    "token": "<token>",
    "scrollback": [
      {
        "from": "player:pc_rowan",
        "address": "table",
        "text": "evening"
      },
      {
        "from": "gm",
        "address": "table",
        "text": "good evening"
      },
      {
        "from": "gm",
        "address": "table",
        "text": "still there?"
      }
    ]
  }
}
```

A GM say arrives in the same shape. `from` is `gm`.

<!-- example: player-hears-gm -->
```json
{
  "seq": 10,
  "body": {
    "kind": "say",
    "from": "gm",
    "address": "table",
    "text": "good evening"
  }
}
```

`say` with no arguments is `SCHEMA`. Say text is at most 4096 characters. Longer text is `SCHEMA`.

<!-- example: player-empty-say -->
```json
{
  "id": "c7",
  "body": {
    "word": "say"
  }
}
```

<!-- example: player-schema -->
```json
{
  "seq": 22,
  "reply_to": "c7",
  "body": {
    "kind": "refused",
    "code": "SCHEMA"
  }
}
```

A word the hello did not list is `SCHEMA`. Nothing is queued. `spook play` does not send it, and prints the `NOT_IN_SPEC` sentence that points at `help`.

<!-- example: player-unknown-word -->
```json
{
  "id": "c6",
  "body": {
    "word": "blah"
  }
}
```

<!-- example: player-word-schema -->
```json
{
  "seq": 21,
  "reply_to": "c6",
  "body": {
    "kind": "refused",
    "code": "SCHEMA"
  }
}
```

A move is a pack verb, with its target in `target`. With a GM seated, the move waits. The reply is `queued`, and the GM hears the same `queued` under the same `seq`. `intent` is the intent id. With no GM seated, the move commits its own round, and the reply comes after that round is written.

<!-- example: player-grit -->
```json
{
  "id": "c4",
  "body": {
    "word": "grit"
  }
}
```

<!-- example: player-queued -->
```json
{
  "seq": 11,
  "reply_to": "c4",
  "body": {
    "kind": "queued",
    "intent": "ed0db120f54c1447842c340c5d523932d1518c796c886f24b1030a9b4a12f73a",
    "actor": "pc_rowan",
    "move": "grit"
  }
}
```

A locked exit is refused before it is queued. `detail` is the pack's locked text.

<!-- example: player-locked-exit -->
```json
{
  "id": "c8",
  "body": {
    "word": "travel",
    "args": {
      "target": "stairs"
    }
  }
}
```

<!-- example: player-no-exit -->
```json
{
  "seq": 23,
  "reply_to": "c8",
  "body": {
    "kind": "refused",
    "code": "NO_EXIT",
    "detail": "The stair door is locked."
  }
}
```

`detail` is omitted when the refusal does not name one. A cost the actor cannot pay is `CANT_AFFORD`, and `detail` is the meter or item id.

Each committed round is one `round` to every seat. Then each seated player receives a `check` for each of their own rolls and a `refused` for each of their own moves the round refused, then a fresh `resume`, then a fresh `view`, then one `lines` for each narrate line they may hear, then one `lines` whose text is a single plain line of the meter changes and items gained or lost that player may know. The change text is omitted when that line is empty. A narrate line is omitted when it would show a fact that player is not allowed to know. An NPC habit refused in a round sends no player message. Empty rounds, with nobody seated, do not leave lines for the next person who sits.

<!-- example: player-round -->
```json
{
  "seq": 12,
  "body": {
    "kind": "round",
    "round_index": 1,
    "world_time": 1085
  }
}
```

The terminal shows the digest once per sitting. The per-round `resume` is for raw clients.

<!-- example: player-check -->
```json
{
  "seq": 13,
  "body": {
    "kind": "check",
    "outcome": "success"
  }
}
```

`outcome` is `success`, `partial`, or `miss`. An NPC's roll sends no player message.

<!-- example: player-changes -->
```json
{
  "seq": 20,
  "body": {
    "kind": "lines",
    "text": "You picked up the Lamp oil."
  }
}
```

A player connection may send 5 messages a second, and it may burst to 10. The claim is one of them. A message past that waits on that connection until the next is allowed. The message is kept. The wait does not delay another seat.

## GM seat

The GM seat is reached at the MCP endpoint. One GM. Initialize claims it. A second GM is `TABLE_FULL`.

POST the initialize body. Send `Accept` and `Content-Type` as in Connect. The response is an SSE event. The JSON-RPC object is the `data` line. Keep the `mcp-session-id` response header on later requests.

<!-- example: initialize-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "method": "initialize",
  "params": {
    "protocolVersion": "2026-07-28",
    "capabilities": {},
    "clientInfo": {
      "name": "seat-protocol",
      "version": "0"
    }
  }
}
```

<!-- example: initialize-result -->
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "result": {
    "protocolVersion": "2025-11-25",
    "capabilities": {
      "tools": {}
    },
    "serverInfo": {
      "name": "rmcp",
      "version": "3.5.1"
    }
  }
}
```

The result's `protocolVersion` is the negotiated version. Use that version on `MCP-Protocol-Version` for the rest of the session, including DELETE. A DELETE that names `2026-07-28` is HTTP 405 and does not release the seat. DELETE with the negotiated version, and the session id, releases it and answers after the seat is free. So does `close`.

Then open one event stream: `GET /mcp` with the `mcp-session-id`, the negotiated `MCP-Protocol-Version` and `Accept: text/event-stream`. Keep it open for the whole session. When it closes, the host ends the session and frees the seat, so a GM process that dies without DELETE frees it once the host sees its connection close. The host prints `gm seat free` on its stdout when it frees the seat. Open only one: the seat ends when any event stream on the session closes. There is no idle timeout, so a GM with no open event stream keeps the seat until it sends DELETE or `close`.

A second initialize, while a GM is sat, does not sit:

<!-- example: gm-session-full -->
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32603,
    "message": "The table is full. [TABLE_FULL]",
    "data": {
      "code": "TABLE_FULL"
    }
  }
}
```

The JSON-RPC error code is the transport's. The seat token is the bracketed word in `message`. The seat token is also `error.data.code`.

### Tools

`tools/list` is the menu. The names, in order, are `table_status`, `perceive`, `queue_intent`, `say`, `commit_round`, and `close`. Any other name is `NOT_IN_SPEC` and writes nothing.

A call is `tools/call`. The body below is the JSON-RPC request. The result examples are the tool's structured content, the object under `result.structuredContent` on the wire. A refusal is that object with a `code`, and the seat stays up.

#### table_status

No key is accepted. The reply carries `pack`, the pack summary. It holds no says: a player catches up on missed messages with a `claim` by token.

<!-- example: table-status-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 2,
  "method": "tools/call",
  "params": {
    "name": "table_status",
    "arguments": {}
  }
}
```

<!-- example: table-status -->
```json
{
  "presence": {
    "players": 1,
    "actors_present": [
      "pc_rowan"
    ]
  },
  "pause": {
    "presence_clocks": "paused"
  },
  "world_time": 1080,
  "round_index": 0,
  "state_rev": 0,
  "pack_rev": "blake2s:5e8aed8ebc0be8c009fdb25a79f4e45c008b4fd081252d985bd23353c8f17f04",
  "last_round_at": "2026-01-01T00:00:00Z",
  "queued": [],
  "last_seq": 0,
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
```

`players` is how many player seats are held. `actors_present` is those PCs. `world_time` is the game clock in minutes. `pack_rev` is the canon hash. A client that opened the pack on disk should compare it and leave on a mismatch. `last_round_at` is the save's stamp. `queued` is waiting moves, each `id`, `actor_id`, and `move`. `last_seq` is the last tape sequence. `pack` is the move menu, cast kinds, place prose, and cast prose. A server built before `pack` moved here leaves it out, and `spook ai-gm` stops with `this spook run is older than spook ai-gm, rebuild both`.

#### perceive

`as` is required. It is a bare actor id. It is not `player:<id>` or `npc:<id>`. A missing `as`, or `as` set to `gm`, is `UNSCOPED_PERCEIVE`. `max_body_chars` may be a non-negative integer. The default budget is 800.

The result is the same object as the player `view` line above.

<!-- example: perceive-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 3,
  "method": "tools/call",
  "params": {
    "name": "perceive",
    "arguments": {
      "as": "pc_rowan"
    }
  }
}
```

An empty argument object:

<!-- example: unscoped-perceive-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 10,
  "method": "tools/call",
  "params": {
    "name": "perceive",
    "arguments": {}
  }
}
```

<!-- example: unscoped-perceive -->
```json
{
  "code": "UNSCOPED_PERCEIVE"
}
```

#### queue_intent

`actor_id` and `move` are required. `actor_id` is a bare actor id. `id`, when sent, becomes the queue id. Omit it and the seat mints one. `target` is a string. `to`, when sent, is a list of actor ids. `round_index` and `rationale_code` may be sent.

The call queues the move. It does not emit `spook/event` and it does not write a player line. The result is the queue id.

<!-- example: intent-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 6,
  "method": "tools/call",
  "params": {
    "name": "queue_intent",
    "arguments": {
      "actor_id": "aldric",
      "move": "wait",
      "id": "gm-wait"
    }
  }
}
```

<!-- example: intent-result -->
```json
{
  "id": "gm-wait"
}
```

A PC a player holds takes only that player's moves. The GM's move for it is `ACTOR_MISMATCH`, and nothing is queued. The claim holds while that player's connection is down, too.

<!-- example: intent-held-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "tools/call",
  "params": {
    "name": "queue_intent",
    "arguments": {
      "actor_id": "pc_rowan",
      "move": "wait"
    }
  }
}
```

<!-- example: intent-held -->
```json
{
  "code": "ACTOR_MISMATCH"
}
```

#### say

`address` and `text` are required. `address` is `table`, `place`, or `actor`. `place` is required when the address is `place`, and `actor` is required when the address is `actor`. `actor` must name an actor. `table` takes neither. A `from` field is `SCHEMA` and appends nothing. The GM speaks as `gm`.

The result is the tape object. The same object follows as a `spook/event`, and each recipient gets a player `say` line.

<!-- example: say-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 4,
  "method": "tools/call",
  "params": {
    "name": "say",
    "arguments": {
      "address": "table",
      "text": "good evening"
    }
  }
}
```

<!-- example: say-result -->
```json
{
  "kind": "say",
  "seq": 2,
  "from": "gm",
  "address": "table",
  "place": null,
  "actor": null,
  "recipients": [
    "pc_rowan"
  ],
  "text": "good evening"
}
```

`place` and `actor` are null when that address does not use them. `recipients` are the seated players who hear the line. The speaker is included when the speaker is a seated player.

#### commit_round

`include_round` is required. `session_id` may be sent; the round does not branch on it.

`include_round: false` while the queue is non-empty is `NOT_IN_SPEC`. The queue stays.

<!-- example: commit-without-round-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 7,
  "method": "tools/call",
  "params": {
    "name": "commit_round",
    "arguments": {
      "include_round": false
    }
  }
}
```

<!-- example: commit-without-round -->
```json
{
  "code": "NOT_IN_SPEC"
}
```

`include_round: true` commits the queue in queue order, one round. An empty queue writes no round file, unless every seated PC is busy, in which case the table fast-forwards.

<!-- example: commit-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 8,
  "method": "tools/call",
  "params": {
    "name": "commit_round",
    "arguments": {
      "include_round": true
    }
  }
}
```

<!-- example: commit-result -->
```json
{
  "state_rev": 1,
  "changed_keys": [
    "clock_paused assassination_debt",
    "last_act_round aldric",
    "last_act_round mira",
    "live_acts_this_hour aldric",
    "live_acts_this_hour mira",
    "note note_mira_door"
  ],
  "journal_path": "state/journal/bundle-0000.jsonl",
  "rolls": [
    {
      "faces": [
        5,
        2
      ],
      "stat": 1,
      "total": 8,
      "target": 0,
      "outcome": "success"
    }
  ]
}
```

`changed_keys` is the state keys the round changed. The GM sees every changed key. A player is told only the changes they can see. `journal_path` is the journal file for that round, or null when the round wrote no round file. `checks` are the rolls. Faces stay on this object and on the GM check event. They do not go to the player.

#### close

No key is accepted. An empty object is legal. A key is `SCHEMA` and the seat stays up. The result is sent after the seat is free. A player line after it commits its own round. A later tool call on that same session is `NOT_IN_SPEC` and writes nothing. Whoever holds the seat then is left as they are.

<!-- example: close-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 11,
  "method": "tools/call",
  "params": {
    "name": "close",
    "arguments": {}
  }
}
```

<!-- example: close-result -->
```json
{
  "released": true
}
```

An unknown tool:

<!-- example: unknown-tool-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 9,
  "method": "tools/call",
  "params": {
    "name": "not_in_spec",
    "arguments": {}
  }
}
```

<!-- example: unknown-tool -->
```json
{
  "code": "NOT_IN_SPEC"
}
```

### Events

The seat pushes one `spook/event` notification for each message the table sends this seat, the hello first. Its params are the message's envelope, as on the game port. A client that ignores them misses those messages. `table_status` returns only the current state. The GM seat is not rate-limited.

A player say:

<!-- example: say-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "seq": 9,
    "body": {
      "kind": "say",
      "from": "player:pc_rowan",
      "address": "table",
      "recipients": [
        "pc_rowan"
      ],
      "text": "evening"
    }
  }
}
```

The GM's own say is the same shape. This one followed the `say` tool:

<!-- example: gm-say-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "seq": 10,
    "body": {
      "kind": "say",
      "from": "gm",
      "address": "table",
      "recipients": [
        "pc_rowan"
      ],
      "text": "good evening"
    }
  }
}
```

A move the player sent while a GM is sat. Its `seq` is the one on the player's reply:

<!-- example: queued-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "seq": 11,
    "body": {
      "kind": "queued",
      "intent": "ed0db120f54c1447842c340c5d523932d1518c796c886f24b1030a9b4a12f73a",
      "actor": "pc_rowan",
      "move": "grit"
    }
  }
}
```

Each committed round. Its rolls, faces and all, are on the `commit_round` result:

<!-- example: round-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "seq": 12,
    "body": {
      "kind": "round",
      "round_index": 1,
      "world_time": 1085
    }
  }
}
```

The GM hears `presence` when a claim starts or ends.

A GM may also sit on `/gm` on the game port, as `spook gm` and `spook ai-gm` do. Its words are the tools' words: `status`, `perceive`, `queue`, `say`, `commit` and `release`, with the tools' arguments. Each reply comes on the same stream as the events, after the messages the word caused, so a `commit` reply follows its `round` messages and a `say` reply is the GM's own copy of the say. A second GM on `/gm` while the seat is held gets one `refused` with `TABLE_FULL`, and the connection closes.

## Errors

A refusal on the game port is a `refused` body, `{"kind":"refused","code":"..."}`, and the connection stays up. `detail` is present only when the refusal names a thing, as in the locked-exit example. On `CHOOSE_PC`, `detail` is a JSON array of the open bare PC ids. A GM tool refusal is `{"code":"..."}` and the seat stays up. The GM object has no `detail`.

Each connection has an outbound queue of at most 256 messages and 64 KiB. A client that stops reading is dropped, and that seat is released. Other seats keep their messages.

| Code | What the client should do |
|---|---|
| `TABLE_FULL` | Stop. Three PCs are claimed, or the GM seat is held. Claim or connect again only after that seat is free. Do not retry in a loop. |
| `SEAT_TAKEN` | That PC is held. Claim a different live PC. |
| `CHOOSE_PC` | The connection has no claim. Claim one id from the JSON list in `detail`. An empty list means no PC is free. |
| `SCHEMA` | The message, the word or the arguments are the wrong shape, or the word is not one the hello listed. Send a corrected message. |
| `NOT_IN_SPEC` | The tool is not in the menu, the verb has no rule that applies, `commit_round` set `include_round` false while a move is queued, or the call is on a seat that was released. Nothing was written for that call. The queue is still there after the false commit. |
| `LINE_TOO_LONG` | The message is over 8 KiB. The connection stays up. Send a shorter one. |
| `UNSCOPED_PERCEIVE` | Call `perceive` again with `as` set to a bare actor id. |
| `NO_EXIT` | The move was not queued. For a player, `detail` is the locked text when the pack set one. |
| `CANT_AFFORD` | The move was not queued. For a player, `detail` is the meter or item id. |
| `MISSING_ITEM` | The actor does not have the item. The move was not queued. |
| `NOT_A_WEAPON` | That item is not a weapon. The move was not queued. |
| `BUSY` | The character is busy. The move was not queued. Send it again when they are free. |
| `UNKNOWN_MOVE` | That actor has no such move. |
| `UNKNOWN_ID` | That id is not in the world. |
| `OUT_OF_RANGE` | A number fell outside its range. The record was left as it was. |
| `PAUSED_CLOCK` | That clock is paused. The change was not kept. |
| `BUDGET` | The change does not fit the round. It was not kept. |
| `INELIGIBLE` | That actor cannot take the action right now. |
| `UNKNOWN_EFFECT` | The effect is not one the engine runs. |
| `ILLEGAL_KNOWLEDGE` | That actor cannot know that. |
| `VIEWER_MISMATCH` | The effect is not for this viewer. |
| `ENGINE_ON_PLAYER` | The table does not take that action for a player. |
| `ACTOR_MISMATCH` | The action is not this actor's. |
| `TABLE_EMPTY` | Nobody is seated. |
| `COMMIT_CONFLICT` | Another process holds the pack lock. Stop. Nothing was written. |
| `HAND_EDIT` | The save was edited by hand. Stop sending moves until it is migrated. |
| `JOURNAL_DIVERGED` | The journal no longer replays to its recorded hash. `spook verify` names the record. Stop. Nothing was written. |
| `STORE_IO` | The disk refused a write, a flush, a rename or a read, including a read of the journal. Stop, and do not resend until the operator fixes the disk. |
| `PACK_REV_MISMATCH` | The save does not match the pack. Stop. |
| `RULES_REV_MISMATCH` | The save was written by different rules. Start a fresh save. This fails at pack open, before a seat exists. |

HTTP statuses, before a seat token:

| Status | What the client should do |
|---|---|
| 400 | `MCP-Protocol-Version` names a version the server does not know. Omit the header, or send a version from a previous handshake. |
| 403 | `Host` is not `127.0.0.1`, or `Origin` is present and is not `http://127.0.0.1` on any port. Dial `http://127.0.0.1:<port>/mcp`. |
| 405 | DELETE named a protocol version that has no session to delete. Send the negotiated `protocolVersion` from the initialize result. |
| 406 | `Accept` did not list both `application/json` and `text/event-stream`. |
| 413 | The POST body is over 1 MiB. Shrink it. The seat was not taken. |
