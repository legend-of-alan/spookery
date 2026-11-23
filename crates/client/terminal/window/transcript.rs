//! The transcript pane: every line the window has shown, wrapped and pinned to its bottom unless
//! the player scrolls it back.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::Palette;

/// Every line the window has shown, oldest first, and how far the player has scrolled it back.
pub(super) struct Transcript {
    lines: Vec<TranscriptLine>,
    /// How many wrapped rows the transcript is scrolled up from its bottom.
    scroll_offset_rows: usize,
    /// How many rows PgUp adds to `scroll_offset_rows` and PgDn takes off: the pane's height less
    /// one, as last drawn.
    scroll_step_rows: usize,
}

/// One line, and whether it is a round divider, which the compact layout leaves out.
struct TranscriptLine {
    // repeats module: it wraps ratatui's `Line`, which this module also names
    line: Line<'static>,
    is_round_divider: bool,
}

impl Transcript {
    pub(super) fn new() -> Self {
        Self {
            lines: Vec::new(),
            scroll_offset_rows: 0,
            scroll_step_rows: 1,
        }
    }

    pub(super) fn line_count(&self) -> usize {
        self.lines.len()
    }

    pub(super) fn push(&mut self, line: Line<'static>) {
        self.lines.push(TranscriptLine {
            line,
            is_round_divider: false,
        });
    }

    pub(super) fn push_round_divider(&mut self, line: Line<'static>) {
        self.lines.push(TranscriptLine {
            line,
            is_round_divider: true,
        });
    }

    /// Puts a blank line before the line at `index`.
    pub(super) fn insert_blank_line(&mut self, index: usize) {
        let blank_line = TranscriptLine {
            line: Line::default(),
            is_round_divider: false,
        };
        self.lines.insert(index, blank_line);
    }

    pub(super) fn scroll_back(&mut self) {
        self.scroll_offset_rows += self.scroll_step_rows;
    }

    pub(super) fn scroll_forward(&mut self) {
        self.scroll_offset_rows = self
            .scroll_offset_rows
            .saturating_sub(self.scroll_step_rows);
    }

    pub(super) fn scroll_to_bottom(&mut self) {
        self.scroll_offset_rows = 0;
    }

    /// The lines, wrapped and pinned to the bottom unless scrolled back, with the round dividers
    /// only when `is_showing_round_dividers`.
    pub(super) fn draw(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        is_showing_round_dividers: bool,
        palette: Palette,
    ) {
        let border = palette.titled_border("Transcript", true);
        let inner_area = border.inner(area);
        let lines: Vec<Line> = (self.lines.iter())
            .filter(|transcript_line| {
                is_showing_round_dividers || !transcript_line.is_round_divider
            })
            .map(|transcript_line| transcript_line.line.clone())
            .collect();
        let transcript_text = Paragraph::new(lines).wrap(Wrap { trim: false });
        let wrapped_rows = transcript_text.line_count(inner_area.width);
        let page_rows = usize::from(inner_area.height);
        self.scroll_step_rows = page_rows.saturating_sub(1).max(1);
        let max_offset_rows = wrapped_rows.saturating_sub(page_rows);
        self.scroll_offset_rows = self.scroll_offset_rows.min(max_offset_rows);
        let top_row = u16::try_from(max_offset_rows - self.scroll_offset_rows).unwrap_or(u16::MAX);
        frame.render_widget(transcript_text.scroll((top_row, 0)).block(border), area);
    }
}
