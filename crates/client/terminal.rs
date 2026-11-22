//! The play terminal: plain lines or the game window.

use tokio::sync::mpsc::UnboundedReceiver;

use crate::protocol::{self, ServerBody, ServerMessage};
use crate::text;
use crate::Error;

mod window;
use window::Window;

/// Where play shows each message and reads each line the player sends, and the last `seq` it
/// showed, so a reconnect asks for what came after.
pub struct Terminal {
    surface: Surface,
    last_seq: u64,
}

/// Plain lines on stdout and stderr with lines typed on stdin, or the game window.
enum Surface {
    Plain(Plain),
    Window(Box<Window>),
}

impl Terminal {
    /// Opens the game window and takes over the terminal until this `Terminal` is dropped.
    pub fn window() -> Result<Self, Error> {
        Ok(Self {
            surface: Surface::Window(Box::new(Window::open()?)),
            last_seq: 0,
        })
    }

    /// Prints plain lines, as text or with `raw` as protocol frames.
    pub fn plain(raw: bool) -> Self {
        Self {
            surface: Surface::Plain(Plain {
                text: text::Renderer::new(),
                raw,
                typed: crate::stdin_lines(),
            }),
            last_seq: 0,
        }
    }

    /// Shows the message and records its `seq`.
    pub(crate) fn show(
        &mut self,
        message: &ServerMessage,
        view: Option<&protocol::View>,
    ) -> Result<(), Error> {
        self.last_seq = self.last_seq.max(message.seq.unwrap_or(0));
        match &mut self.surface {
            Surface::Plain(plain) if plain.raw => {
                plain.print_raw(message);
                Ok(())
            }
            Surface::Plain(plain) => {
                plain.print(&message.body);
                Ok(())
            }
            Surface::Window(window) => window.show(&message.body, view),
        }
    }

    /// The next line the player sends: typed on stdin, or chosen or typed in the window.
    /// `None` when stdin ends or the player quits the window.
    pub(crate) async fn next_line(
        &mut self,
        view: Option<&protocol::View>,
        commands: &[String],
    ) -> Result<Option<String>, Error> {
        match &mut self.surface {
            Surface::Plain(plain) => Ok(plain.typed.recv().await),
            Surface::Window(window) => window.next_line(view, commands).await,
        }
    }

    /// Whether refusals show as their bare code.
    pub(crate) fn is_raw(&self) -> bool {
        matches!(&self.surface, Surface::Plain(plain) if plain.raw)
    }

    pub(crate) fn last_seq(&self) -> u64 {
        self.last_seq
    }
}

/// The plain surface. Game text goes to stdout and refusals go to stderr. With `raw`,
/// each message other than the picker or a refusal is its protocol frame.
struct Plain {
    text: text::Renderer,
    raw: bool,
    typed: UnboundedReceiver<String>,
}

impl Plain {
    fn print(&mut self, body: &ServerBody) {
        let shown = match body {
            ServerBody::Hello {
                open,
                taken,
                pc_names,
                ..
            } => text::render_pc_picker(&text::pc_picker_rows(open, taken, pc_names)),
            ServerBody::Refused(refusal) => {
                eprintln!("{}", text::render_refusal(refusal, self.raw));
                return;
            }
            _ => self.text.render(body),
        };
        if !shown.is_empty() {
            println!("{shown}");
        }
    }

    /// The picker as text, a refusal as its code on stderr, and any other message as its frame.
    fn print_raw(&mut self, message: &ServerMessage) {
        match &message.body {
            ServerBody::Hello { .. } | ServerBody::Refused(_) => self.print(&message.body),
            _ => println!("{}", message.to_text()),
        }
    }
}
