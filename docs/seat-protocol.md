# Seat protocol

This page is the seat protocol for a player client or a GM integration. `spook run` is the only server. It owns both seats. [Engine guarantees](engine-guarantees.md) is what the engine promises. This page is how to speak to it.

The examples are one live sitting of `packs/oneshot-docks`. Rowan is the player, in the shop, at game time 1080. A test plays that sitting and fails when a fenced example and the table disagree. The `id` on a tool call is the caller's. The ports on the listening line are the ports that run chose.

A tick is game time. A round is one batch of queued moves.

## Connect

The game port is TCP. It accepts player hellos only. The default is `127.0.0.1:9737`.

The agent door is streamable HTTP at `http://127.0.0.1:<door>/mcp`. When the game port is fixed, the door is that port plus one, so the default door is `http://127.0.0.1:9738/mcp`. `--addr 127.0.0.1:0` asks the operating system for the game port, and the door is its own bind. `--door 127.0.0.1:<port>` overrides the door, including port 0. Do not add one to a port-0 game port. Read the listening line.

Both binds are loopback. Any other host is refused before the pack opens.

After both sockets are bound, and before the table serves, `spook run` prints one line on stdout:

<!-- example: listening -->
```json
{
  "kind": "listening",
  "game": "127.0.0.1:61762",
  "door": "127.0.0.1:61763"
}
```

`game` and `door` are `127.0.0.1` and a port. They are different ports. This run happened to draw two adjacent ports. The next run draws its own.

The door requires:

- `Host: 127.0.0.1`. `localhost` is 403.
- `Origin`, when the request sends one, is `http://127.0.0.1` on any port. Anything else is 403. A missing `Origin` is accepted.
- `Accept` lists both `application/json` and `text/event-stream`.
- `Content-Type: application/json` on a POST.
- A body of at most 1 MiB. A longer body is 413 and does not take the seat.
- `MCP-Protocol-Version`, when present, names a version the server knows. An unknown version is 400. A missing header is accepted.

There is no account and no connection secret.

## Player seat

Dial the game address. Send one line, then read one JSON object per line. Lines are UTF-8 and end in a newline. The player socket has no GM field and no roll faces.

### Hello

The hello is the actor id with a `player:` prefix. It is not JSON. `spook play --as player:pc_rowan` sends this line. The docks pack has three live PCs: `pc_rowan`, `pc_harbormouse`, and `pc_wren`. An empty line is not a hello. The table refuses it with `CHOOSE_PC`. `missing` is a JSON array of the free bare PC ids. When none are free, `missing` is `[]`. `player:` alone is a bad id and is `SCHEMA`, not an empty hello.

<!-- example: player-hello -->
```text
player:pc_rowan
```

A legal hello sits that PC. Speech already on the tape for that player arrives first, and each of those `say` lines has `"replay": true`. This sitting has none. The table then sends a `resume` line and a `perceive` line.

<!-- example: player-resume -->
```json
{
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
```

<!-- example: player-perceive -->
```json
{
  "kind": "view",
  "viewer": "pc_rowan",
  "place": "docks_shop",
  "place_name": "Docks Shop",
  "body": "The shop faces the water. Tins sit in a row.",
  "visible_actors": [
    "aldric",
    "pc_rowan"
  ],
  "actor_names": [
    "Aldric",
    "Rowan"
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
```

`resume` carries the notes addressed to that viewer (`notes`) and their own clocks (`clocks`). `view` is what that viewer can see. `viewer` is the bare actor id, the same form `as` takes. It is not `player:<id>` or `npc:<id>`. `place_name` and `actor_names` are the pack's display names. `body` is the place body. `meters` is that viewer's own meters. A hidden meter is left out. `value` is the current number and `max` is the cap. `exits` is omitted when the place has none. `status` is omitted unless the viewer is still travelling, and then it is `on the stairs until 21:40`. Fact bodies are the place prose the viewer is allowed to read.

Sitting keeps presence in memory and does not bump `state_rev`. The cap is three player seats, in the host, not in the pack.

A refused hello is one JSON line. The seat then closes its write side.

The PC is already held:

<!-- example: player-seat-taken -->
```json
{
  "kind": "refused",
  "code": "SEAT_TAKEN"
}
```

A fourth player, after three are sat:

<!-- example: player-session-full -->
```json
{
  "kind": "refused",
  "code": "TABLE_FULL"
}
```

An id that is not a live PC:

<!-- example: player-unknown-pc -->
```json
{
  "kind": "refused",
  "code": "SCHEMA"
}
```

An empty line, after Rowan is sat. `detail` is the free PCs:

<!-- example: player-choose-pc -->
```json
{
  "kind": "refused",
  "code": "CHOOSE_PC",
  "detail": [
    "pc_harbormouse",
    "pc_wren"
  ]
}
```

The game port is not the GM seat. The line `gm` is refused and nothing is written:

<!-- example: player-gm-on-game-port -->
```json
{
  "kind": "refused",
  "code": "NOT_IN_SPEC"
}
```

A hello or a later line over 8 KiB, not counting the newline, closes the socket:

<!-- example: player-line-too-long -->
```json
{
  "kind": "refused",
  "code": "LINE_TOO_LONG"
}
```

A hello that is `SCHEMA`, `CHOOSE_PC`, or `LINE_TOO_LONG` closes. Invalid UTF-8 is `SCHEMA`, not a replacement character. A hello that does not finish within 10 seconds closes, and nothing is committed. A `SCHEMA` refusal on a line after the hello leaves the socket up.

### Lines the player sends

After the hello, each command is one text line. The server does not wrap it in JSON.

`look`, `help`, and `map` do not take a round. The reply is `lines`. `text` is the words to print, with newlines already in the string. `look` reprints the place name, its prose, and the exits. `help` lists each verb whose `if` holds, then those exits. An exit is `stairs (locked) — 2 min`. A destination is not printed.

<!-- example: player-look -->
```text
look
```

<!-- example: player-lines -->
```json
{
  "kind": "lines",
  "text": "Docks Shop\nThe shop faces the water. Tins sit in a row.\n\nExits:\ndocks_street — 0 min\nstairs (locked) — 2 min"
}
```

`say` is speech. It appends the tape and does not take a round. The first word after `say` is the address when it is `place`, or an actor id, and more words follow. `place` is the speaker's place. An actor id is that actor. Any other line is address `table`.

<!-- example: player-say -->
```text
say evening
```

The speaker hears the line back:

<!-- example: player-say-frame -->
```json
{
  "kind": "say",
  "from": "player:pc_rowan",
  "address": "table",
  "text": "evening"
}
```

A live `say` omits `replay`. Sitting again sends the earlier lines first, with `replay` set. The terminal prints them under one `Earlier:` heading and leaves a blank line before the next live frame.

<!-- example: player-replay-say -->
```json
{
  "kind": "say",
  "from": "player:pc_rowan",
  "address": "table",
  "text": "evening",
  "replay": true
}
```

A GM say arrives in the same shape. `from` is `gm`.

<!-- example: player-hears-gm -->
```json
{
  "kind": "say",
  "from": "gm",
  "address": "table",
  "text": "good evening"
}
```

`say` with no words is `SCHEMA`. The socket stays up.

<!-- example: player-empty-say -->
```text
say
```

<!-- example: player-schema -->
```json
{
  "kind": "refused",
  "code": "SCHEMA"
}
```

Say text is at most 4096 characters. Longer text is `SCHEMA`.

A word that is not `say`, `help`, `look`, `map`, or a move is `NOT_IN_SPEC`. Nothing is queued.

<!-- example: player-unknown-line -->
```text
blah
```

<!-- example: player-not-in-spec -->
```json
{
  "kind": "refused",
  "code": "NOT_IN_SPEC"
}
```

A move is the verb and its target, in the pack's words. With no GM seated, the line commits its own round. With a GM seated, the line waits. The player receives `queued`. The id is the intent id.

<!-- example: player-grit -->
```text
grit
```

<!-- example: player-queued -->
```json
{
  "kind": "queued",
  "id": "ed0db120f54c1447842c340c5d523932d1518c796c886f24b1030a9b4a12f73a"
}
```

A locked exit is refused before it is queued. `detail` is the pack's locked text.

<!-- example: player-locked-exit -->
```text
travel stairs
```

<!-- example: player-no-exit -->
```json
{
  "kind": "refused",
  "code": "NO_EXIT",
  "detail": "The stair door is locked."
}
```

`detail` is omitted when the refusal does not name one. A cost the actor cannot pay is `CANT_AFFORD`, and `detail` is the meter or item id.

A refusal during the round arrives as `refused` before the fresh `resume`. The round still commits. An NPC habit refused in that round sends no player frame. The GM seat gets no faces for that refusal.

When the GM commits the round, every seated player receives a fresh `resume`, then a fresh `view`, then one `lines` frame for each narrate line they may hear, then one `lines` frame. Its text is a single plain line of the meter changes and items gained or lost that player may know. A player who rolled also receives `check` before the `resume`. The change frame is omitted when that line is empty. A narrate line is omitted when it would show a fact that player is not allowed to know. Empty rounds, with nobody seated, do not leave lines for the next person who sits.

The terminal shows the digest once per sitting. The per-round `resume` is for raw clients.

<!-- example: player-check -->
```json
{
  "kind": "check",
  "outcome": "success"
}
```

`outcome` is `success`, `partial`, or `miss`. An NPC's roll sends no player frame.

<!-- example: player-changes -->
```json
{
  "kind": "lines",
  "text": "You picked up the Lamp oil."
}
```

`close` leaves the seat. It has no reply.

A player connection may send 5 lines a second, and it may burst to 10. A line past that waits on that connection until the next line is allowed. The line is kept. The wait does not delay another seat.

## GM seat

The GM seat is the agent door. One GM. Initialize claims it. A second GM is `TABLE_FULL`.

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

Then open one event stream: `GET /mcp` with the `mcp-session-id`, the negotiated `MCP-Protocol-Version` and `Accept: text/event-stream`. Keep it open for the whole session. When it closes, the host ends the session and frees the seat, so a GM process that dies without DELETE frees it as soon as its connection drops. Open only one: the seat ends when any event stream on the session closes. There is no idle timeout, so a GM with no open event stream keeps the seat until it sends DELETE or `close`.

A second initialize, while a GM is sat, does not sit:

<!-- example: gm-session-full -->
```json
{
  "jsonrpc": "2.0",
  "id": 1,
  "error": {
    "code": -32603,
    "message": "The table is full. [TABLE_FULL]"
  }
}
```

The JSON-RPC error code is the transport's. The seat token is the bracketed word in `message`.

### Tools

`tools/list` is the menu. The names, in order, are `table_status`, `perceive`, `queue_intent`, `say`, `commit_round`, and `close`. Any other name is `NOT_IN_SPEC` and writes nothing.

A call is `tools/call`. The body below is the JSON-RPC request. The result examples are the tool's structured content, the object under `result.structuredContent` on the wire. A refusal is that object with a `code`, and the seat stays up.

#### table_status

No key is required. `since` is the only other accepted key. It is a tape sequence number. The reply then includes `events`, the say objects after that sequence, inside the tape window. The window defaults to 64 lines and stops at 256. A generic client that ignores `spook/event` uses this as the pull path. Without `since`, the reply also carries `pack`, the pack summary.

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

`players` is how many player seats are held. `actors_present` is those PCs. `world_time` is the game clock in minutes. `pack_rev` is the canon hash. A client that opened the pack on disk should compare it and leave on a mismatch. `last_round_at` is the save's stamp. `queued` is waiting moves, each `id`, `actor_id`, and `move`. `last_seq` is the last tape sequence. `events` is absent unless `since` was sent. `pack` is the move menu, cast kinds, place prose, and cast prose, and it is absent when `since` was sent. A server built before `pack` moved here leaves it out, and `spook ai-gm` stops with `this spook run is older than spook ai-gm, rebuild both`.

After the two says above, `since` 0 returns them:

<!-- example: table-status-since-request -->
```json
{
  "jsonrpc": "2.0",
  "id": 5,
  "method": "tools/call",
  "params": {
    "name": "table_status",
    "arguments": {
      "since": 0
    }
  }
}
```

<!-- example: table-status-since -->
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
  "last_seq": 2,
  "events": [
    {
      "kind": "say",
      "seq": 1,
      "from": "player:pc_rowan",
      "address": "table",
      "place": null,
      "actor": null,
      "recipients": [
        "pc_rowan"
      ],
      "text": "evening"
    },
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
  ]
}
```

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
      "actor_id": "pc_rowan",
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
    "last_act_round mira",
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

The seat pushes `spook/event` notifications. A generic client may ignore them and poll `table_status` with `since` instead. The GM seat is not rate-limited.

A player say:

<!-- example: say-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "kind": "say",
    "seq": 1,
    "from": "player:pc_rowan",
    "address": "table",
    "place": null,
    "actor": null,
    "recipients": [
      "pc_rowan"
    ],
    "text": "evening"
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
}
```

A move the player sent while a GM is sat:

<!-- example: queued-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "kind": "queued",
    "id": "ed0db120f54c1447842c340c5d523932d1518c796c886f24b1030a9b4a12f73a",
    "actor_id": "pc_rowan",
    "move": "grit"
  }
}
```

A roll, after `commit_round`. The same faces are on the tool result.

<!-- example: check-event -->
```json
{
  "jsonrpc": "2.0",
  "method": "spook/event",
  "params": {
    "kind": "check",
    "faces": [
      5,
      2
    ],
    "stat": 1,
    "total": 8,
    "target": 0,
    "outcome": "success"
  }
}
```

## Errors

A player refusal is `{"kind":"refused","code":"..."}`. `detail` is present only when the refusal names a thing, as in the locked-exit example. On `CHOOSE_PC`, `detail` is a JSON array of the free bare PC ids. A GM tool refusal is `{"code":"..."}` and the seat stays up. The GM object has no `detail`.

A hello refusal is one line, then the write side closes. `LINE_TOO_LONG` closes on a hello and on a later line. `SCHEMA` closes on a hello and leaves a seated player up. Invalid UTF-8 is `SCHEMA`.

Each player socket, and the HTTP GM session, has an outbound queue of at most 256 frames and 64 KiB. A client that stops reading is dropped, and that seat is released. Other seats keep their frames.

| Code | What the client should do |
|---|---|
| `TABLE_FULL` | Stop. Three players are sat, or the GM seat is held. Connect again only after that seat is free. Do not retry the same hello in a loop. |
| `SEAT_TAKEN` | That PC is held. Connect again as a different live PC. |
| `CHOOSE_PC` | The hello was empty. Pick one id from the JSON list in `detail` and connect again as `player:<id>`. An empty list means no PC is free. |
| `SCHEMA` | The line or the arguments are the wrong shape. `player:` with an empty id is `SCHEMA`. On a seated player, send a corrected line. On hello, connect again with `player:<id>` of a live PC. |
| `NOT_IN_SPEC` | The word is not a player command, the tool is not in the menu, `commit_round` set `include_round` false while a move is queued, or the call is on a session that already closed. Nothing was written for that call. The queue is still there after the false commit. |
| `LINE_TOO_LONG` | The line is over 8 KiB, not counting the newline. The socket is closed. Connect again with a shorter line. |
| `UNSCOPED_PERCEIVE` | Call `perceive` again with `as` set to a bare actor id. |
| `NO_EXIT` | The move was not queued. On the player line, `detail` is the locked text when the pack set one. |
| `CANT_AFFORD` | The move was not queued. On the player line, `detail` is the meter or item id. |
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
| `COMMIT_CONFLICT` | The table was already writing. Send `commit_round` again. |
| `HAND_EDIT` | The save was edited by hand. Stop sending moves until it is migrated. |
| `PACK_REV_MISMATCH` | The save does not match the pack. Stop. |
| `RULES_REV_MISMATCH` | The save was written by different rules. Start a fresh save. This fails at pack open, before a seat exists. |

HTTP statuses, before a seat token:

| Status | What the client should do |
|---|---|
| 400 | `MCP-Protocol-Version` names a version the server does not know. Omit the header, or send a version from a previous handshake. |
| 403 | `Host` is not `127.0.0.1`, or `Origin` is present and is not `http://127.0.0.1` on any port. Dial `http://127.0.0.1:<door>/mcp`. |
| 405 | DELETE named a protocol version that has no session to delete. Send the negotiated `protocolVersion` from the initialize result. |
| 406 | `Accept` did not list both `application/json` and `text/event-stream`. |
| 413 | The POST body is over 1 MiB. Shrink it. The seat was not taken. |
