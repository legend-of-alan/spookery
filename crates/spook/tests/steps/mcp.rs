use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::Arc;
use std::time::{Duration, Instant};

use cucumber::{given, then, when};
use rmcp::model::{
    CallToolRequestParams, CallToolResult, ClientCapabilities, Implementation, ProtocolVersion,
};
use rmcp::transport::StreamableHttpClientTransport;
use rmcp::ServiceExt;
use serde_json::{json, Map, Value};
use std::net::Shutdown;
use std::sync::Mutex;

use crate::support::{self, Proc};
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

/// The GM that holds the seat in a cross-door scenario. It lives across steps,
/// and the after hook drops it if a step fails first.
enum Holder {
    Tcp,
    Agent(rmcp::service::RunningService<rmcp::RoleClient, rmcp::model::ClientConfig>),
}

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

#[when(regex = r"^a GM holds the seat over (HTTP|TCP)$")]
fn gm_holds_over(world: &mut World, door: String) {
    let (_tcp, http) = listen_http(table::pack(world));
    world.mark = usize::from(http);
    let holder = match door.as_str() {
        "TCP" => {
            table::seat_gm();
            Holder::Tcp
        }
        "HTTP" => Holder::Agent(block_on(async move {
            let running = connect_http(http)
                .await
                .unwrap_or_else(|err| panic!("HTTP GM: {err}"));
            let status = call(&running, "session_status", None).await;
            assert_sat(&status, "session_status");
            running
        })),
        other => panic!("unknown door {other}"),
    };
    *HOLDER.lock().unwrap_or_else(|err| err.into_inner()) = Some(holder);
}

#[when(regex = r"^another GM tries to sit over (HTTP|TCP)$")]
fn another_gm_tries(world: &mut World, door: String) {
    world.scratch = match door.as_str() {
        "TCP" => table::second_hello("gm"),
        "HTTP" => {
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
            raw
        }
        other => panic!("unknown door {other}"),
    };
}

#[then(expr = "the newcomer is told the table is full")]
fn newcomer_is_told_full(world: &mut World) {
    assert!(
        world.scratch.contains("SESSION_FULL"),
        "refusal:\n{}",
        world.scratch
    );
}

#[when(regex = r"^the GM over (HTTP|TCP) leaves$")]
fn holder_leaves(_world: &mut World, _door: String) {
    let holder = HOLDER
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .take()
        .unwrap_or_else(|| panic!("no GM holds the seat"));
    match holder {
        Holder::Tcp => {
            let write = crate::with_live(|live| live.gm_write.take())
                .unwrap_or_else(|| panic!("no TCP GM"));
            // The reader thread holds a clone of this socket, so shut it down
            // rather than only dropping this handle.
            let _ = write.shutdown(Shutdown::Both);
        }
        Holder::Agent(running) => {
            block_on(async move {
                running
                    .cancel()
                    .await
                    .unwrap_or_else(|err| panic!("close GM session: {err}"));
            });
        }
    }
}

#[then(regex = r"^a GM can sit over (HTTP|TCP)$")]
fn a_gm_can_sit_over(world: &mut World, door: String) {
    let start = Instant::now();
    let mut last = String::new();
    while start.elapsed() < Duration::from_secs(5) {
        let sat = match door.as_str() {
            "TCP" => tcp_gm_sits(),
            "HTTP" => agent_sits(http_port(world)),
            other => panic!("unknown door {other}"),
        };
        match sat {
            Ok(()) => return,
            Err(err) => last = err,
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("no GM could sit over {door}: {last}");
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

fn tcp_gm_sits() -> Result<(), String> {
    let port = crate::with_live(|live| live.port);
    let mut stream =
        TcpStream::connect(("127.0.0.1", port)).map_err(|err| format!("connect: {err}"))?;
    stream
        .write_all(b"gm\n{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2024-11-05\",\"capabilities\":{},\"clientInfo\":{\"name\":\"proof\",\"version\":\"0\"}}}\n")
        .map_err(|err| format!("gm hello: {err}"))?;
    let line = table::read_line(&mut stream);
    let _ = stream.shutdown(Shutdown::Both);
    if line.contains("\"id\":1") && line.contains("\"result\"") {
        Ok(())
    } else {
        Err(line)
    }
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
    let tcp = table::free_port();
    let http = loop {
        let port = table::free_port();
        if port != tcp {
            break port;
        }
    };
    let pack_arg = support::pack_arg(pack);
    let addr = format!("127.0.0.1:{tcp}");
    let http_addr = format!("127.0.0.1:{http}");
    let mut server = support::spawn(&["run", &pack_arg, "--addr", &addr, "--mcp-http", &http_addr]);
    wait_port(tcp, &mut server);
    wait_port(http, &mut server);
    let opened_now = std::fs::read(pack.join("state/now.yaml"))
        .unwrap_or_else(|err| panic!("read now.yaml: {err}"));
    crate::set_live(table::Live {
        port: tcp,
        player_write: None,
        player_text: Arc::new(std::sync::Mutex::new(String::new())),
        players: Vec::new(),
        gm_write: None,
        gm_text: Arc::new(std::sync::Mutex::new(String::new())),
        opened_now: opened_now.clone(),
        seated_now: opened_now,
        seated_rev: 0,
        server: Some(server),
    });
    (tcp, http)
}

fn wait_port(port: u16, server: &mut Proc) {
    let start = Instant::now();
    let addr = format!("127.0.0.1:{port}");
    loop {
        if TcpStream::connect(&addr).is_ok() {
            return;
        }
        if let Some(status) = server
            .child
            .try_wait()
            .unwrap_or_else(|err| panic!("wait: {err}"))
        {
            panic!(
                "spook run exited {status} before {addr} listened\nstderr:\n{}",
                support::text_of(&server.stderr)
            );
        }
        if start.elapsed() > Duration::from_secs(5) {
            panic!(
                "{addr} did not listen\nstderr:\n{}",
                support::text_of(&server.stderr)
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn http_post(port: u16, host: &str, origin: Option<&str>) -> String {
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
