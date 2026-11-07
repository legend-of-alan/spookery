//! A scripted model and the AI GM that plays with it, run in this process on a paused clock.

use std::collections::VecDeque;
use std::io::Write;
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use serde_json::{json, Value};
use spookery::agent::{parse_completion, ChatMessage, CompletionError, Model, ToolCall};
use spookery::ai_gm;

use crate::support;
use crate::World;

use super::table;

/// How long a held model call waits on the AI GM's paused clock.
pub const HOLD: Duration = Duration::from_secs(1);

/// A model that answers each call with the next scripted tool calls, read through
/// `parse_completion` the way `HttpModel` reads a reply. With no script left it answers with
/// its `repeat` calls, none unless set.
#[derive(Clone, Default)]
pub struct ScriptedModel {
    script: Arc<Mutex<Script>>,
}

#[derive(Default)]
struct Script {
    replies: VecDeque<Result<String, CompletionError>>,
    /// How many replies were ever scripted.
    scripted: usize,
    /// The reply once the script runs out, none for a reply with no calls.
    repeat: Option<String>,
    /// How many of the next calls wait `HOLD` before they answer.
    held: usize,
    calls: Vec<Vec<ChatMessage>>,
}

impl ScriptedModel {
    /// One reply that makes `calls`, each a tool name and its arguments. No calls is done.
    pub fn push(&self, calls: &[(&str, &str)]) {
        self.push_reply(Ok(render_completion(calls)));
    }

    pub fn push_error(&self, err: CompletionError) {
        self.push_reply(Err(err));
    }

    fn push_reply(&self, reply: Result<String, CompletionError>) {
        let mut script = self.script.lock().unwrap();
        script.replies.push_back(reply);
        script.scripted += 1;
    }

    /// The calls every reply makes once the script runs out.
    pub fn repeat(&self, calls: &[(&str, &str)]) {
        self.script.lock().unwrap().repeat = Some(render_completion(calls));
    }

    pub fn hold_next(&self) {
        self.script.lock().unwrap().held += 1;
    }

    /// The messages of each call so far.
    pub fn calls(&self) -> Vec<Vec<ChatMessage>> {
        self.script.lock().unwrap().calls.clone()
    }

    /// Waits until the model was called once past its script: the turn that read the last
    /// scripted reply has asked again, and was told it is done.
    pub fn wait_past_script(&self) -> Vec<Vec<ChatMessage>> {
        table::poll_until("a model call past the script", || {
            let script = self.script.lock().unwrap();
            if script.calls.len() > script.scripted {
                Ok(script.calls.clone())
            } else {
                Err(script.calls.len())
            }
        })
    }
}

impl Model for ScriptedModel {
    async fn complete(&self, messages: &[ChatMessage]) -> Result<Vec<ToolCall>, CompletionError> {
        let (reply, held) = {
            let mut script = self.script.lock().unwrap();
            script.calls.push(messages.to_vec());
            let held = script.held > 0;
            script.held = script.held.saturating_sub(1);
            let repeat = script
                .repeat
                .clone()
                .unwrap_or_else(|| render_completion(&[]));
            (script.replies.pop_front().unwrap_or(Ok(repeat)), held)
        };
        if held {
            tokio::time::sleep(HOLD).await;
        }
        parse_completion(&reply?)
    }
}

/// A chat-completions reply that makes `calls`, each a tool name and its arguments.
pub fn render_completion(calls: &[(&str, &str)]) -> String {
    let calls: Vec<Value> = calls
        .iter()
        .enumerate()
        .map(|(index, (name, arguments))| {
            json!({
                "id": format!("call_{index}"),
                "type": "function",
                "function": { "name": name, "arguments": arguments }
            })
        })
        .collect();
    json!({
        "id": "stub",
        "object": "chat.completion",
        "choices": [{
            "index": 0,
            "message": { "role": "assistant", "content": "prose", "tool_calls": calls }
        }]
    })
    .to_string()
}

/// The tool results a model call carried, in order.
pub fn read_tool_results(messages: &[ChatMessage]) -> Vec<String> {
    messages
        .iter()
        .filter_map(|message| match message {
            ChatMessage::Tool { content, .. } => Some(content.clone()),
            _ => None,
        })
        .collect()
}

/// `ai_gm::run` on its own thread, on a current-thread runtime whose clock starts paused.
/// A blocking task held for the run keeps tokio from auto-advancing, so paused time moves
/// only on `advance`.
pub struct AiGm {
    /// What the run wrote to `err`, and then its error if it returned one, as `spook ai-gm`
    /// prints it.
    pub err: Arc<Mutex<String>>,
    advance: tokio::sync::mpsc::UnboundedSender<table::Advance>,
    pub thread: thread::JoinHandle<Result<(), spookery_client::Error>>,
}

impl AiGm {
    /// Moves the AI GM's paused clock forward, after the AI GM has read what was ready for
    /// it. Timers due by then have fired when this returns.
    pub fn advance(&self, by: Duration) {
        let ended = || panic!("the AI GM ended\n{}", support::text_of(&self.err));
        let (ack, acked) = tokio::sync::oneshot::channel();
        self.advance.send((by, ack)).unwrap_or_else(|_| ended());
        support::block_on(acked).unwrap_or_else(|_| ended());
    }
}

/// Starts the AI GM with `model` and returns once it has written `listening`.
pub fn start_ai_gm(world: &World, model: impl Model) -> AiGm {
    let addr: SocketAddr = table::game_addr(world).parse().unwrap();
    let (read, mut write) = std::io::pipe().unwrap_or_else(|err| panic!("pipe: {err}"));
    let err = Arc::new(Mutex::new(String::new()));
    support::drain(read, Arc::clone(&err));
    let (advance, mut advances) = tokio::sync::mpsc::unbounded_channel::<table::Advance>();
    let thread = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .start_paused(true)
            .build()
            .unwrap_or_else(|err| panic!("runtime: {err}"));
        let report = write
            .try_clone()
            .unwrap_or_else(|err| panic!("pipe: {err}"));
        let result = runtime.block_on(async {
            let (_hold, held) = std::sync::mpsc::channel::<()>();
            tokio::task::spawn_blocking(move || held.recv());
            let run = ai_gm::run(addr, model, report);
            tokio::pin!(run);
            loop {
                tokio::select! {
                    // `run` first, so an advance lands after it has read what is ready.
                    biased;
                    result = &mut run => break result,
                    Some((by, ack)) = advances.recv() => {
                        tokio::time::advance(by).await;
                        let _ = ack.send(());
                    }
                }
            }
        });
        if let Err(err) = &result {
            let _ = writeln!(write, "{err}");
        }
        result
    });
    let gm = AiGm {
        err,
        advance,
        thread,
    };
    table::wait_suffix(&gm.err, 0, "listening");
    gm
}

/// Stops the host, which closes the AI GM's connection, and waits for the AI GM to end.
pub fn stop_ai_gm(world: &mut World, gm: AiGm) {
    table::live_mut(world).host.take();
    let _ = gm.thread.join();
}
