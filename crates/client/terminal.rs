//! The play terminal: plain lines or the game window.

use tokio::sync::mpsc::UnboundedReceiver;

use crate::protocol::{self, ServerBody, ServerMessage};
use crate::text;
use crate::Error;

#[cfg(test)]
pub(crate) mod test_surface;
mod window;
use window::Window;

/// Where play shows each message and reads each line the player sends, and the last `seq` it
/// received, so a reconnect asks for what came after.
pub struct Terminal {
    surface: Surface,
    last_seq: u64,
}

/// Plain lines on stdout and stderr with lines typed on stdin, or the game window.
enum Surface {
    Plain(Plain),
    Window(Box<Window>),
    #[cfg(test)]
    Test(test_surface::TestSurface),
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
                renderer: text::Renderer::new(),
                is_raw: raw,
                stdin_lines: crate::stdin_lines(),
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
            Surface::Plain(plain) => {
                plain.print(message);
                Ok(())
            }
            Surface::Window(window) => window.show(message, view),
            #[cfg(test)]
            Surface::Test(surface) => {
                surface.send(test_surface::SurfaceCall::Show(message.clone()));
                Ok(())
            }
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
            Surface::Plain(plain) => Ok(plain.stdin_lines.recv().await),
            Surface::Window(window) => window.next_line(view, commands).await,
            #[cfg(test)]
            Surface::Test(surface) => Ok(surface.next_line().await),
        }
    }

    /// Records the `seq` of the reply to play's own `map` and fills the window's place panel
    /// from it, keeping it out of the transcript. A refusal of that `map` shows nowhere.
    pub(crate) fn fill_place_panel(
        &mut self,
        message: &ServerMessage,
        view: Option<&protocol::View>,
    ) -> Result<(), Error> {
        self.last_seq = self.last_seq.max(message.seq.unwrap_or(0));
        match (&mut self.surface, &message.body) {
            (Surface::Window(window), ServerBody::Map { places }) => {
                window.fill_place_panel(places, view)
            }
            #[cfg(test)]
            (Surface::Test(surface), _) => {
                surface.send(test_surface::SurfaceCall::FillPlacePanel(message.clone()));
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// Whether the surface has a place panel, which lists the places past the exits.
    pub(crate) fn has_place_panel(&self) -> bool {
        match self.surface {
            Surface::Plain(_) => false,
            Surface::Window(_) => true,
            #[cfg(test)]
            Surface::Test(_) => true,
        }
    }

    /// Whether refusals show as their bare code.
    pub(crate) fn is_raw(&self) -> bool {
        matches!(&self.surface, Surface::Plain(plain) if plain.is_raw)
    }

    pub(crate) fn last_seq(&self) -> u64 {
        self.last_seq
    }
}

/// The plain surface. Game text goes to stdout and refusals go to stderr. With `is_raw`,
/// each message other than the picker or a refusal is its protocol frame.
struct Plain {
    renderer: text::Renderer,
    is_raw: bool,
    stdin_lines: UnboundedReceiver<String>,
}

impl Plain {
    /// The picker as text, a refusal on stderr, and any other message as its frame or its text.
    fn print(&mut self, message: &ServerMessage) {
        let body = &message.body;
        let content = match body {
            ServerBody::Hello {
                open,
                taken,
                pc_names,
                ..
            } => text::render_pc_picker(&text::pc_picker_rows(open, taken, pc_names)),
            ServerBody::Refused(refusal) => {
                eprintln!("{}", text::render_refusal(refusal, self.is_raw));
                return;
            }
            _ if self.is_raw => message.to_text(),
            _ => self.renderer.render(body),
        };
        if !content.is_empty() {
            println!("{content}");
        }
    }
}
