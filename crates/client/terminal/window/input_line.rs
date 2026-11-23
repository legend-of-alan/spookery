//! The input line: what the player types behind its prompt, the lines they sent for Up and Down,
//! and Tab completion of command names.

use crossterm::event::{Event, KeyEvent};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::widgets::{Block, BorderType, Paragraph};
use ratatui::Frame;
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use super::{Palette, FLAME, MULBERRY, PARCHMENT};

#[derive(Default)]
pub(super) struct InputLine {
    input: Input,
    /// The lines the player sent, oldest first.
    history: Vec<String>,
    /// The index in `history` of the line Up and Down put in the input, `None` while the player
    /// types a new line.
    history_index: Option<usize>,
}

impl InputLine {
    /// Takes the typed line and keeps it in the history. `None` when it is blank.
    pub(super) fn take_line(&mut self) -> Option<String> {
        let line = self.input.value_and_reset();
        self.history_index = None;
        if line.trim().is_empty() {
            return None;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        Some(line)
    }

    /// Types the key into the line.
    pub(super) fn handle_key(&mut self, key: KeyEvent) {
        self.input.handle_event(&Event::Key(key));
    }

    /// Completes the first word to the command names that start with it: to the one name and a
    /// space, or to the prefix all of them share.
    pub(super) fn complete_command(&mut self, commands: &[String]) {
        let typed_word = self.input.value();
        if typed_word.contains(' ') {
            return;
        }
        let mut matches = commands
            .iter()
            .filter(|command| command.starts_with(typed_word));
        let Some(first_match) = matches.next() else {
            return;
        };
        let mut common_prefix = first_match.as_str();
        let mut is_unique = true;
        for command in matches {
            is_unique = false;
            let prefix_bytes: usize = common_prefix
                .chars()
                .zip(command.chars())
                .take_while(|(left, right)| left == right)
                .map(|(letter, _)| letter.len_utf8())
                .sum();
            common_prefix = &common_prefix[..prefix_bytes];
        }
        let completion = if is_unique {
            format!("{common_prefix} ")
        } else {
            common_prefix.to_string()
        };
        self.input = Input::new(completion);
    }

    /// Puts an older line from the history in the input, or a newer one, or clears it past the
    /// newest.
    pub(super) fn recall_history(&mut self, is_older: bool) {
        if self.history.is_empty() {
            return;
        }
        let newest_index = self.history.len() - 1;
        self.history_index = match (self.history_index, is_older) {
            (None, true) => Some(newest_index),
            (Some(index), true) => Some(index.saturating_sub(1)),
            (Some(index), false) if index < newest_index => Some(index + 1),
            (_, false) => None,
        };
        let line = self.history_index.map(|index| self.history[index].clone());
        self.input = Input::new(line.unwrap_or_default());
    }

    /// The line behind its prompt, with the cursor where the player types.
    pub(super) fn draw(&self, frame: &mut Frame, area: Rect, palette: Palette) {
        let border = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(palette.style(MULBERRY));
        let [prompt_area, value_area] =
            Layout::horizontal([Constraint::Length(2), Constraint::Fill(1)])
                .areas(border.inner(area));
        frame.render_widget(border, area);
        frame.render_widget(Paragraph::new(palette.span("› ", FLAME)), prompt_area);
        let scroll_cells = self
            .input
            .visual_scroll(usize::from(value_area.width.saturating_sub(1)));
        let input_span = palette.span(self.input.value().to_string(), PARCHMENT);
        let scroll_x = u16::try_from(scroll_cells).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(input_span).scroll((0, scroll_x)), value_area);
        let cursor_x = u16::try_from(self.input.visual_cursor() - scroll_cells).unwrap_or(u16::MAX);
        frame.set_cursor_position((value_area.x.saturating_add(cursor_x), value_area.y));
    }
}
