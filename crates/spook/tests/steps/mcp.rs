use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use std::sync::Mutex;

use crate::support;
use crate::World;

use super::table;

#[when(expr = "a coding agent calls the GM tools over loopback HTTP")]
fn agent_calls_over_http(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    block_on(exercise_http(http));
}

#[when(expr = "the GM seat is held and another agent opens HTTP")]
fn second_http(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    let refusal = block_on(async move {
        let first = connect_http(http)
            .await
            .unwrap_or_else(|err| panic!("first HTTP GM: {err}"));
        let status = call(&first, "session_status", None).await;
        assert_sat(&status, "session_status");
        let raw = http_post(http, "127.0.0.1", None);
        if connect_http(http).await.is_ok() {
            panic!("a second HTTP GM sat\n{raw}");
        }
        let _ = first.cancel().await;
        raw
    });
    world.stderr = refusal;
    world.stdout.clear();
}

#[when(expr = "the operator runs with an open MCP HTTP address")]
fn open_http_address(world: &mut World) {
    let pack = support::pack_arg(table::pack(world));
    table::run(world, &["run", &pack, "--mcp-http", "0.0.0.0:9"]);
}

#[when(expr = "an HTTP request arrives with a public Origin")]
fn public_origin(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.scratch = http_post(http, "127.0.0.1", Some("https://evil.example"));
}

#[then(expr = "the response is forbidden")]
fn the_response_is_forbidden(world: &mut World) {
    let status = world
        .scratch
        .lines()
        .next()
        .unwrap_or_else(|| panic!("no HTTP status\n{}", world.scratch));
    assert!(
        status.contains(" 403 "),
        "status: {status}\n{}",
        world.scratch
    );
}

#[then(expr = "a GM hello still sits")]
fn a_gm_hello_still_sits(_world: &mut World) {
    table::seat_gm();
}

#[when(expr = "an HTTP GM session ends")]
fn http_session_ends(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.mark = usize::from(http);
    block_on(async move {
        let running = connect_http(http)
            .await
            .unwrap_or_else(|err| panic!("HTTP GM: {err}"));
        let status = call(&running, "session_status", None).await;
        assert_sat(&status, "session_status");
        running
            .cancel()
            .await
            .unwrap_or_else(|err| panic!("close HTTP session: {err}"));
    });
}

#[then(expr = "another GM can sit over HTTP")]
fn another_gm_can_sit(world: &mut World) {
    let http = u16::try_from(world.mark).unwrap_or_else(|_| panic!("no HTTP port"));
    block_on(async move {
        let start = Instant::now();
        loop {
            match connect_http(http).await {
                Ok(running) => {
                    let status = call(&running, "session_status", None).await;
                    assert_sat(&status, "session_status");
                    let _ = running.cancel().await;
                    return;
                }
                Err(err) if start.elapsed() > Duration::from_secs(5) => {
                    panic!("the next GM did not sit: {err}");
                }
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        }
    });
}

#[when(regex = r"^an HTTP request arrives with Host (\S+)$")]
fn request_with_host(world: &mut World, host: String) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.scratch = http_post(http, &host, None);
}

#[when(expr = "a human GM sits on the default agent door")]
fn a_human_gm_sits_on_the_default_agent_door(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    let addr = table::game_addr();
    let door = table::door_addr();
    let pack_arg = support::pack_arg(&pack);
    world.gm = Some(support::spawn(&[
        "gm",
        &pack_arg,
        "--addr",
        &addr,
        "--mcp-http",
        &door,
    ]));
    world.mark = usize::from(crate::with_live(|live| live.http_port));
    let start = Instant::now();
    loop {
        let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
        let stdout = support::text_of(&gm.stdout);
        if stdout.contains("Commands:") {
            break;
        }
        if let Some(status) = gm.child.try_wait().unwrap_or_else(|err| panic!("{err}")) {
            panic!("gm exited {status:?}\n{}", support::text_of(&gm.stderr));
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!("gm did not sit\n{stdout}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[when(expr = "a coding agent tries to sit that door")]
fn a_coding_agent_tries_to_sit_that_door(world: &mut World) {
    let http = u16::try_from(world.mark).unwrap_or_else(|_| panic!("no HTTP port"));
    world.scratch = http_post(http, "127.0.0.1", None);
}

#[then(expr = "the agent is told the table is full")]
fn the_agent_is_told_the_table_is_full(world: &mut World) {
    assert!(
        world.scratch.contains("SESSION_FULL"),
        "refusal:\n{}",
        world.scratch
    );
}

#[when(expr = "the human GM leaves")]
fn the_human_gm_leaves(world: &mut World) {
    let gm = world.gm.as_mut().unwrap_or_else(|| panic!("no gm"));
    let mut stdin = gm.stdin.take().unwrap_or_else(|| panic!("no gm stdin"));
    writeln!(stdin, "quit").unwrap_or_else(|err| panic!("quit: {err}"));
    drop(stdin);
    let gm = world.gm.as_mut().unwrap();
    let status = gm
        .child
        .wait()
        .unwrap_or_else(|err| panic!("wait gm: {err}"));
    assert!(status.success(), "gm quit exited {status:?}");
}

#[then(expr = "a coding agent can sit that door")]
fn a_coding_agent_can_sit_that_door(world: &mut World) {
    let http = u16::try_from(world.mark).unwrap_or_else(|_| panic!("no HTTP port"));
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(5) {
        let raw = http_post(http, "127.0.0.1", None);
        if raw.contains("\"protocolVersion\"") {
            return;
        }
        last = raw;
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("the agent did not sit\n{last}");
}

#[when(expr = "spook run listens with no MCP flag")]
fn spook_run_listens_with_no_mcp_flag(world: &mut World) {
    let pack = table::pack(world).to_path_buf();
    table::listen(&pack);
    world.mark = usize::from(crate::with_live(|live| live.http_port));
}

#[then(expr = "the reported agent door serves \\/mcp")]
fn the_reported_agent_door_serves_mcp(world: &mut World) {
    let http = u16::try_from(world.mark).unwrap_or_else(|_| panic!("no door port"));
    let raw = http_post(http, "127.0.0.1", None);
    assert!(
        raw.lines().next().unwrap_or("").contains(" 200 "),
        "no /mcp on {http}\n{raw}"
    );
    assert!(raw.contains("\"protocolVersion\""), "{raw}");
}

#[when(expr = "an HTTP request arrives with an unknown MCP-Protocol-Version")]
fn an_unknown_protocol_version(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.scratch = http_post_version(http, "1999-01-01");
}

#[when(expr = "an HTTP request arrives with the negotiated MCP-Protocol-Version")]
fn the_negotiated_protocol_version(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.scratch = http_post_version(http, ProtocolVersion::LATEST.as_str());
}

#[then(expr = "the response status is {int}")]
fn the_response_status_is(world: &mut World, status: usize) {
    let line = world
        .scratch
        .lines()
        .next()
        .unwrap_or_else(|| panic!("no HTTP status\n{}", world.scratch));
    let needle = format!(" {status} ");
    assert!(line.contains(&needle), "status: {line}\n{}", world.scratch);
}

#[then(expr = "the response is accepted")]
fn the_response_is_accepted(world: &mut World) {
    let status = world
        .scratch
        .lines()
        .next()
        .unwrap_or_else(|| panic!("no HTTP status\n{}", world.scratch));
    assert!(
        status.contains(" 200 "),
        "status: {status}\n{}",
        world.scratch
    );
    assert!(
        world.scratch.contains("\"protocolVersion\""),
        "no initialize result\n{}",
        world.scratch
    );
}

/// The HTTP GM that holds the seat across steps. The after hook drops it.
type Holder = rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig>;

static HOLDER: Mutex<Option<Holder>> = Mutex::new(None);

pub fn drop_holder() {
    let holder = HOLDER.lock().unwrap_or_else(|err| err.into_inner()).take();
    drop(holder);
}

fn http_port(world: &World) -> u16 {
    u16::try_from(world.mark).unwrap_or_else(|_| panic!("no HTTP port"))
}

#[given(expr = "a docks table with a seated player and an open HTTP door")]
fn seated_player_on_the_http_door(world: &mut World) {
    let pack = support::temp_pack("http-say");
    let (_tcp, http) = listen_http(&pack);
    table::seat_player(&pack);
    world.mark = usize::from(http);
    world.pack = Some(pack);
}

#[when(expr = "a GM holds the seat over HTTP")]
fn gm_holds_over_http(world: &mut World) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.mark = usize::from(http);
    let holder = block_on(async move {
        let running = connect_http(http)
            .await
            .unwrap_or_else(|err| panic!("HTTP GM: {err}"));
        let status = call(&running, "session_status", None).await;
        assert_sat(&status, "session_status");
        running
    });
    *HOLDER.lock().unwrap_or_else(|err| err.into_inner()) = Some(holder);
}

#[when(expr = "another GM tries to sit over HTTP")]
fn another_gm_tries_http(world: &mut World) {
    // rmcp's client error drops the JSON-RPC message, so read the
    // refusal from a raw initialize, then check a real client fails too.
    let http = http_port(world);
    let raw = http_post(http, "127.0.0.1", None);
    let sat = block_on(async move {
        match connect_http(http).await {
            Ok(running) => {
                let _ = running.cancel().await;
                true
            }
            Err(_) => false,
        }
    });
    assert!(!sat, "a second GM sat over HTTP\n{raw}");
    world.scratch = raw;
}

#[then(expr = "the newcomer is told the table is full")]
fn newcomer_is_told_full(world: &mut World) {
    assert!(
        world.scratch.contains("SESSION_FULL"),
        "refusal:\n{}",
        world.scratch
    );
}

#[when(expr = "the GM over HTTP leaves")]
fn holder_leaves(_world: &mut World) {
    let running = HOLDER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
        .unwrap_or_else(|| panic!("no GM holds the seat"));
    block_on(async move {
        running
            .cancel()
            .await
            .unwrap_or_else(|err| panic!("close GM session: {err}"));
    });
}

#[then(expr = "a GM can sit over HTTP")]
fn a_gm_can_sit_over_http(world: &mut World) {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(5) {
        match agent_sits(http_port(world)) {
            Ok(()) => return,
            Err(err) => last = err,
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no GM could sit over HTTP: {last}");
}

fn agent_sits(port: u16) -> Result<(), String> {
    block_on(async move {
        let running = connect_http(port).await.map_err(|err| format!("{err:?}"))?;
        let status = call(&running, "session_status", None).await;
        assert_sat(&status, "session_status");
        let _ = running.cancel().await;
        Ok(())
    })
}

#[then(expr = "world time moved forward")]
fn world_time_moved_forward(world: &mut World) {
    let now = table::now_of(table::pack(world));
    assert!(now.world_time > 1080, "world_time {}", now.world_time);
}

#[then(expr = "the refusal names SESSION_FULL")]
fn the_refusal_names_session_full(world: &mut World) {
    assert!(world.stderr.contains("SESSION_FULL"), "{}", world.stderr);
    assert!(world.stdout.is_empty(), "{}", world.stdout);
}

#[when(expr = "a coding agent says good evening over HTTP")]
fn agent_says_over_http(world: &mut World) {
    let http = http_port(world);
    let mark = table::player_len();
    block_on(async move {
        let running = connect_http(http)
            .await
            .unwrap_or_else(|err| panic!("HTTP: {err}"));
        let said = call(
            &running,
            "say",
            Some(json!({"address": "table", "text": "good evening"})),
        )
        .await;
        assert_sat(&said, "say");
        let _ = running.cancel().await;
    });
    world.player_added = table::wait_player_from(mark, "{\"kind\":\"say\"");
}

fn block_on<T: Send>(work: impl std::future::Future<Output = T> + Send) -> T {
    tokio::task::block_in_place(|| {
        tokio::runtime::Handle::current().block_on(async move {
            tokio::time::timeout(Duration::from_secs(20), work)
                .await
                .unwrap_or_else(|_| panic!("the agent timed out"))
        })
    })
}

async fn exercise_http(port: u16) {
    let running = connect_http(port)
        .await
        .unwrap_or_else(|err| panic!("HTTP: {err}"));
    exercise(&running).await;
    let _ = running.cancel().await;
}

async fn exercise(running: &impl ToolClient) {
    for (name, arguments) in [
        ("session_status", None),
        ("perceive", Some(json!({"as": "player:pc_rowan"}))),
        (
            "intent",
            Some(json!({
                "actor_id": "pc_rowan",
                "actor_kind": "player",
                "source": "player",
                "move": "wait"
            })),
        ),
        (
            "say",
            Some(json!({"address": "table", "text": "good evening"})),
        ),
        ("commit_turn", Some(json!({"include_tick": true}))),
    ] {
        let result = call(running, name, arguments).await;
        assert_sat(&result, name);
    }
}

trait ToolClient {
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ServiceError>> + Send;
}

impl<S> ToolClient for rmcp::service::RunningService<rmcp::RoleClient, S>
where
    S: rmcp::Service<rmcp::RoleClient> + Send + Sync,
{
    fn call_tool(
        &self,
        params: CallToolRequestParams,
    ) -> impl std::future::Future<Output = Result<CallToolResult, rmcp::ServiceError>> + Send {
        rmcp::service::RunningService::call_tool(self, params)
    }
}

async fn call(running: &impl ToolClient, name: &str, arguments: Option<Value>) -> CallToolResult {
    let params = match arguments {
        Some(value) => CallToolRequestParams::new(name.to_string()).with_arguments(object(value)),
        None => CallToolRequestParams::new(name.to_string()),
    };
    running
        .call_tool(params)
        .await
        .unwrap_or_else(|err| panic!("{name}: {err}"))
}

fn object(value: Value) -> Map<String, Value> {
    value
        .as_object()
        .unwrap_or_else(|| panic!("arguments are not an object: {value}"))
        .clone()
}

fn assert_sat(result: &CallToolResult, name: &str) {
    let body = result.structured_content.clone().unwrap_or(Value::Null);
    assert!(body.get("code").is_none(), "{name} was refused: {body}");
    assert_ne!(result.is_error, Some(true), "{name}: {body}");
}

fn client_config() -> rmcp::model::ClientConfig {
    rmcp::model::ClientConfig::new(
        ClientCapabilities::default(),
        Implementation::new("spook-promises", "0"),
    )
    .with_protocol_version(ProtocolVersion::LATEST)
}

async fn connect_http(
    port: u16,
) -> Result<
    rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig>,
    rmcp::service::ClientInitializeError,
> {
    let transport = StreamableHttpClientTransport::from_uri(format!("http://127.0.0.1:{port}/mcp"));
    client_config().serve(transport).await
}

fn listen_http(pack: &std::path::Path) -> (u16, u16) {
    table::listen_flags(
        pack,
        &["--addr", "127.0.0.1:0", "--mcp-http", "127.0.0.1:0"],
        &[],
    )
}

fn http_post_version(port: u16, version: &str) -> String {
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"probe\",\"version\":\"0\"}}}}}}"
    );
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nMCP-Protocol-Version: {version}\r\nContent-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|err| panic!("connect HTTP: {err}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}

pub fn http_post(port: u16, host: &str, origin: Option<&str>) -> String {
    let version = ProtocolVersion::LATEST.as_str();
    let body = format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{{\"protocolVersion\":\"{version}\",\"capabilities\":{{}},\"clientInfo\":{{\"name\":\"probe\",\"version\":\"0\"}}}}}}"
    );
    let origin = match origin {
        Some(origin) => format!("Origin: {origin}\r\n"),
        None => String::new(),
    };
    let request = format!(
        "POST /mcp HTTP/1.1\r\nHost: {host}:{port}\r\n{origin}Content-Type: application/json\r\nAccept: application/json, text/event-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).unwrap_or_else(|err| panic!("connect HTTP: {err}"));
    stream
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap_or_else(|err| panic!("timeout: {err}"));
    stream
        .write_all(request.as_bytes())
        .unwrap_or_else(|err| panic!("write HTTP: {err}"));
    let mut buf = String::new();
    let _ = stream.read_to_string(&mut buf);
    buf
}
