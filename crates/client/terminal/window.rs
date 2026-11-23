//! The game window `spook play` opens at a terminal: the PC picker, then the transcript, the
//! place panel, the input line and the key bar, painted in the Spookery palette. `Terminal`
//! hands it each message body and reads from it each line the player sends.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::protocol::{self, Outcome, ServerBody, Speaker, Speech};
use crate::text;
use crate::Error;

/// One color of the palette: its truecolor value and its 256-color fallback.
#[derive(Clone, Copy)]
struct PaletteColor {
    rgb: u32,
    ansi_index: u8,
}

impl PaletteColor {
    const fn new(rgb: u32, ansi_index: u8) -> Self {
        Self { rgb, ansi_index }
    }

    fn to_color(self, is_truecolor: bool) -> Color {
        if is_truecolor {
            let [_, r, g, b] = self.rgb.to_be_bytes();
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(self.ansi_index)
        }
    }
}

const VOID: PaletteColor = PaletteColor::new(0x000000, 16);
const GOLD: PaletteColor = PaletteColor::new(0xffca51, 221);
const AMBER: PaletteColor = PaletteColor::new(0xffb43d, 215);
const ORANGE: PaletteColor = PaletteColor::new(0xff9d35, 208);
const TANGERINE: PaletteColor = PaletteColor::new(0xf98835, 209);
const EMBER: PaletteColor = PaletteColor::new(0xf27539, 202);
const FLAME: PaletteColor = PaletteColor::new(0xe76648, 203);
const SCARLET: PaletteColor = PaletteColor::new(0xd85b5b, 167);
const ROSE: PaletteColor = PaletteColor::new(0xce4f6d, 168);
const BERRY: PaletteColor = PaletteColor::new(0xca3a7d, 162);
const PLUM: PaletteColor = PaletteColor::new(0xb7308b, 163);
const DRIP: PaletteColor = PaletteColor::new(0xa21d85, 126);
const PARCHMENT: PaletteColor = PaletteColor::new(0xd9c79f, 187);
const ASH: PaletteColor = PaletteColor::new(0x8c8270, 101);
const DUSK: PaletteColor = PaletteColor::new(0x1e0f21, 234);

/// The title bar's letters, one gradient band each, gold to plum.
const TITLE_LETTERS: [(char, PaletteColor); 8] = [
    ('S', GOLD),
    ('P', AMBER),
    ('O', ORANGE),
    ('O', TANGERINE),
    ('K', FLAME),
    ('E', SCARLET),
    ('R', BERRY),
    ('Y', PLUM),
];

/// How many cells a meter's gauge takes in the place panel.
const GAUGE_CELLS: usize = 10;

/// The terminal in raw mode on the alternate screen, its event stream and the screen drawn on
/// it. Dropping it restores the terminal.
pub(crate) struct Window {
    terminal: DefaultTerminal,
    event_stream: EventStream,
    screen: Screen,
}

impl Window {
    /// Sets up the terminal, which `ratatui` also restores when play panics. Truecolor when
    /// `COLORTERM` says so or the terminal is Windows Terminal, else the 256-color palette.
    pub(crate) fn open() -> Result<Self, Error> {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let is_truecolor = matches!(colorterm.as_str(), "truecolor" | "24bit")
            || std::env::var_os("WT_SESSION").is_some();
        let terminal = ratatui::try_init().map_err(|err| {
            ratatui::restore();
            Error::Io(err)
        })?;
        Ok(Self {
            terminal,
            event_stream: EventStream::new(),
            screen: Screen::new(is_truecolor),
        })
    }

    /// Adds one body to the screen and draws it.
    pub(crate) fn show(
        &mut self,
        body: &ServerBody,
        view: Option<&protocol::View>,
    ) -> Result<(), Error> {
        self.screen.show(body);
        self.draw(view)
    }

    /// Handles keys until the player sends a line: a PC id at the PC picker, else a typed line.
    /// `None` when they quit with Ctrl+C.
    pub(crate) async fn next_line(
        &mut self,
        view: Option<&protocol::View>,
        commands: &[String],
    ) -> Result<Option<String>, Error> {
        self.draw(view)?;
        while let Some(event) = self.event_stream.next().await {
            let key_request = match event.map_err(Error::Io)? {
                Event::Key(key) => self.screen.on_key(key, commands),
                _ => None,
            };
            self.draw(view)?;
            match key_request {
                Some(KeyRequest::Send(line)) => return Ok(Some(line)),
                Some(KeyRequest::Quit) => return Ok(None),
                None => {}
            }
        }
        Ok(None)
    }

    fn draw(&mut self, view: Option<&protocol::View>) -> Result<(), Error> {
        let screen = &mut self.screen;
        self.terminal
            .draw(|frame| screen.draw(frame, view))
            .map_err(Error::Io)?;
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// What a key asks play to do, from `Screen::on_key` to `Window::next_line`.
#[derive(Debug, PartialEq)]
enum KeyRequest {
    /// Send this line, as the plain surface sends a typed one.
    Send(String),
    /// Release the PC and leave.
    Quit,
}

/// What the window shows: the PC picker until a PC is chosen, then the title bar, transcript,
/// place panel and input line.
struct Screen {
    is_truecolor: bool,
    pc_picker: Option<PcPicker>,
    /// The last round's index for the title bar, `None` before the first round.
    last_round_index: Option<u64>,
    /// The world time after the last round.
    world_time_ticks: u64,
    renderer: text::Renderer,
    /// Every line the window has shown in the transcript pane, oldest first.
    transcript: Vec<Line<'static>>,
    /// How many wrapped rows the transcript is scrolled up from its bottom.
    scroll_offset_rows: usize,
    /// How many rows PgUp adds to `scroll_offset_rows` and PgDn takes off: the transcript
    /// pane's height less one, as last drawn.
    scroll_step_rows: usize,
    /// The place whose name and prose the transcript showed last.
    last_place_id: String,
    input: Input,
    /// The lines the player sent, oldest first.
    history: Vec<String>,
    /// The index in `history` of the line Up and Down put in the input, `None` while the
    /// player types a new line.
    history_index: Option<usize>,
}

/// Every PC the hello lists, and the open one Enter would choose.
struct PcPicker {
    rows: Vec<text::PcPickerRow>,
    list_state: ListState,
}

impl PcPicker {
    /// Moves the selection to the open PC above or below with Up and Down. Enter returns the
    /// selected PC's id.
    fn on_key(&mut self, code: KeyCode) -> Option<String> {
        let selected_index = self.list_state.selected()?;
        let is_open = |index: &usize| self.rows[*index].open_id.is_some();
        let next_index = match code {
            KeyCode::Up => (0..selected_index).rev().find(is_open),
            KeyCode::Down => (selected_index + 1..self.rows.len()).find(is_open),
            KeyCode::Enter => return self.rows[selected_index].open_id.clone(),
            _ => None,
        };
        if next_index.is_some() {
            self.list_state.select(next_index);
        }
        None
    }
}

impl Screen {
    fn new(is_truecolor: bool) -> Self {
        Self {
            is_truecolor,
            pc_picker: None,
            last_round_index: None,
            world_time_ticks: 0,
            renderer: text::Renderer::new(),
            transcript: Vec::new(),
            scroll_offset_rows: 0,
            scroll_step_rows: 1,
            last_place_id: String::new(),
            input: Input::default(),
            history: Vec::new(),
            history_index: None,
        }
    }

    fn style(&self, color: PaletteColor) -> Style {
        Style::new().fg(color.to_color(self.is_truecolor))
    }

    fn span(&self, content: impl Into<String>, color: PaletteColor) -> Span<'static> {
        Span::styled(content.into(), self.style(color))
    }

    /// Adds each line of `content` to the transcript in `color`.
    fn push_text(&mut self, content: &str, color: PaletteColor) {
        let style = self.style(color);
        let lines = content
            .lines()
            .map(|line| Line::styled(line.to_string(), style));
        self.transcript.extend(lines);
    }

    fn show(&mut self, body: &ServerBody) {
        match body {
            ServerBody::Hello {
                open,
                taken,
                pc_names,
                ..
            } => {
                let rows = text::pc_picker_rows(open, taken, pc_names);
                let first_open_index = rows.iter().position(|row| row.open_id.is_some());
                let list_state = ListState::default().with_selected(first_open_index);
                self.pc_picker = Some(PcPicker { rows, list_state });
            }
            ServerBody::Round {
                round_index,
                world_time,
            } => {
                self.last_round_index = Some(*round_index);
                self.world_time_ticks = *world_time;
                let divider = format!("── round {round_index} · tick {world_time} ──");
                self.transcript
                    .push(Line::styled(divider, self.style(DRIP)));
            }
            ServerBody::View(view) => {
                if view.place != self.last_place_id {
                    self.last_place_id = view.place.clone();
                    self.push_place_arrival(view);
                }
            }
            ServerBody::Say(speech) => {
                let line = self.render_speech(speech);
                self.transcript.push(line);
            }
            ServerBody::Check { outcome } => {
                let color = match outcome {
                    Outcome::Success => GOLD,
                    Outcome::Partial => EMBER,
                    Outcome::Miss => SCARLET,
                };
                let sentence = text::render(body);
                self.transcript
                    .push(Line::styled(format!("◆ {sentence}"), self.style(color)));
            }
            ServerBody::Refused(refusal) => {
                self.push_text(&text::render_refusal(refusal, false), ROSE);
            }
            _ => {
                let color = match body {
                    ServerBody::Changes { .. } | ServerBody::Queued { .. } => ASH,
                    _ => PARCHMENT,
                };
                let content = self.renderer.render(body);
                self.push_text(&content, color);
            }
        }
    }

    /// The place's name and prose, shown once when the PC arrives.
    fn push_place_arrival(&mut self, view: &protocol::View) {
        if !view.place_name.is_empty() {
            let name = self.span(view.place_name.clone(), GOLD);
            let marker = self.span("▌ ", FLAME);
            self.transcript
                .push(Line::from(vec![marker, name.add_modifier(Modifier::BOLD)]));
        }
        self.push_text(view.body.trim(), PARCHMENT);
    }

    /// The speaker's label in their color, then what they said.
    fn render_speech(&self, speech: &Speech) -> Line<'static> {
        let speaker_color = match speech.from {
            Speaker::Player(_) => AMBER,
            Speaker::Gm => ROSE,
            Speaker::Npc(_) => TANGERINE,
        };
        Line::from(vec![
            self.span(text::speaker_label(speech), speaker_color),
            self.span(format!(": {}", speech.text), PARCHMENT),
        ])
    }

    /// Handles a key event and returns what it asks play to do. Only presses count, since
    /// Windows also reports releases.
    fn on_key(&mut self, key: KeyEvent, commands: &[String]) -> Option<KeyRequest> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(KeyRequest::Quit);
        }
        if let Some(pc_picker) = &mut self.pc_picker {
            let pc_id = pc_picker.on_key(key.code)?;
            self.pc_picker = None;
            return Some(KeyRequest::Send(pc_id));
        }
        match key.code {
            KeyCode::Enter => return self.send_input(),
            KeyCode::Tab => self.complete_command(commands),
            KeyCode::Up | KeyCode::Down => self.recall_history(key.code == KeyCode::Up),
            KeyCode::PageUp => self.scroll_offset_rows += self.scroll_step_rows,
            KeyCode::PageDown => {
                self.scroll_offset_rows = self
                    .scroll_offset_rows
                    .saturating_sub(self.scroll_step_rows);
            }
            _ => {
                self.input.handle_event(&Event::Key(key));
            }
        }
        None
    }

    /// Sends the input line, keeps it in the history and echoes it in the transcript.
    fn send_input(&mut self) -> Option<KeyRequest> {
        let line = self.input.value_and_reset();
        self.history_index = None;
        if line.trim().is_empty() {
            return None;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        let echo = Line::from(vec![self.span("› ", FLAME), self.span(line.clone(), BERRY)]);
        self.transcript.push(echo);
        self.scroll_offset_rows = 0;
        Some(KeyRequest::Send(line))
    }

    /// Completes the first word to the command names that start with it: to the one name and a
    /// space, or to the prefix all of them share.
    fn complete_command(&mut self, commands: &[String]) {
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
    fn recall_history(&mut self, is_older: bool) {
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

    fn draw(&mut self, frame: &mut Frame, view: Option<&protocol::View>) {
        let area = frame.area();
        let void = VOID.to_color(self.is_truecolor);
        frame.render_widget(Block::new().style(Style::new().bg(void)), area);
        let [title_area, rule_area, middle_area, input_area, key_bar_area] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_title(frame, title_area, view);
        self.draw_rule(frame, rule_area);
        if self.pc_picker.is_some() {
            self.draw_pc_picker(frame, middle_area);
            self.draw_key_bar(
                frame,
                key_bar_area,
                &[("↑↓", "choose"), ("Enter", "play"), ("Ctrl+C", "quit")],
            );
            return;
        }
        let [transcript_area, place_area] =
            Layout::horizontal([Constraint::Fill(2), Constraint::Fill(1)]).areas(middle_area);
        self.draw_transcript(frame, transcript_area);
        self.draw_place(frame, place_area, view);
        self.draw_input(frame, input_area);
        self.draw_key_bar(
            frame,
            key_bar_area,
            &[
                ("Enter", "send"),
                ("Tab", "complete"),
                ("↑↓", "history"),
                ("PgUp PgDn", "scroll"),
                ("Ctrl+C", "quit"),
            ],
        );
    }

    /// A pane's rounded border with `title` on it: plum with an amber title when
    /// `is_highlighted`, else drip with a tangerine title.
    fn pane_block(&self, title: &str, is_highlighted: bool) -> Block<'static> {
        let (border_color, title_color) = if is_highlighted {
            (PLUM, AMBER)
        } else {
            (DRIP, TANGERINE)
        };
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(border_color))
            .title(self.span(format!(" {title} "), title_color))
    }

    /// SPOOKERY in its gradient, then the PC's name from `view`, the last round and the tick.
    fn draw_title(&self, frame: &mut Frame, area: Rect, view: Option<&protocol::View>) {
        let dusk = DUSK.to_color(self.is_truecolor);
        let mut letter_spans: Vec<Span> = vec![Span::raw(" ")];
        letter_spans.extend(TITLE_LETTERS.iter().map(|(letter, color)| {
            self.span(letter.to_string(), *color)
                .add_modifier(Modifier::BOLD)
        }));
        let mut pc_label = view
            .and_then(|view| text::actor_ids_and_names(view).find(|(id, _)| **id == view.viewer))
            .map(|(_, name)| name.clone())
            .unwrap_or_default();
        if let Some(round_index) = self.last_round_index {
            let world_time_ticks = self.world_time_ticks;
            pc_label.push_str(&format!(" · round {round_index} · tick {world_time_ticks}"));
        }
        let [letters_area, pc_area] =
            Layout::horizontal([Constraint::Length(10), Constraint::Fill(1)]).areas(area);
        frame.render_widget(
            Paragraph::new(Line::from(letter_spans)).bg(dusk),
            letters_area,
        );
        let pc_line = Line::styled(format!("{pc_label} "), self.style(PARCHMENT)).right_aligned();
        frame.render_widget(Paragraph::new(pc_line).bg(dusk), pc_area);
    }

    /// A plum rule of upper half blocks with drip-colored drops hanging from it.
    fn draw_rule(&self, frame: &mut Frame, area: Rect) {
        let plum = PLUM.to_color(self.is_truecolor);
        let drip = DRIP.to_color(self.is_truecolor);
        let void = VOID.to_color(self.is_truecolor);
        let spans: Vec<Span> = (0..area.width)
            .map(|column| {
                let background = if (column * 5 + 3) % 13 < 2 {
                    drip
                } else {
                    void
                };
                Span::styled("▀", Style::new().fg(plum).bg(background))
            })
            .collect();
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_pc_picker(&mut self, frame: &mut Frame, area: Rect) {
        let block = self.pane_block(text::PC_PICKER_HEADING, true);
        let highlight_style = self.style(GOLD).add_modifier(Modifier::BOLD);
        let (open_style, taken_style) = (self.style(PARCHMENT), self.style(ASH));
        let Some(pc_picker) = &mut self.pc_picker else {
            return;
        };
        let mut items: Vec<ListItem> = pc_picker
            .rows
            .iter()
            .map(|row| {
                let style = if row.open_id.is_some() {
                    open_style
                } else {
                    taken_style
                };
                ListItem::new(row.label()).style(style)
            })
            .collect();
        if pc_picker.list_state.selected().is_none() {
            items.push(ListItem::new(text::PC_PICKER_NONE_OPEN).style(taken_style));
        }
        let list = List::new(items)
            .block(block)
            .highlight_style(highlight_style)
            .highlight_symbol("› ");
        frame.render_stateful_widget(list, area, &mut pc_picker.list_state);
    }

    /// The transcript, wrapped and pinned to the bottom unless scrolled back.
    fn draw_transcript(&mut self, frame: &mut Frame, area: Rect) {
        let block = self.pane_block("Transcript", true);
        let inner_area = block.inner(area);
        let paragraph = Paragraph::new(self.transcript.clone()).wrap(Wrap { trim: false });
        let wrapped_rows = paragraph.line_count(inner_area.width);
        let page_rows = usize::from(inner_area.height);
        self.scroll_step_rows = page_rows.saturating_sub(1).max(1);
        let max_offset_rows = wrapped_rows.saturating_sub(page_rows);
        self.scroll_offset_rows = self.scroll_offset_rows.min(max_offset_rows);
        let top_row = u16::try_from(max_offset_rows - self.scroll_offset_rows).unwrap_or(u16::MAX);
        frame.render_widget(paragraph.scroll((top_row, 0)).block(block), area);
    }

    /// Who is here, the exits, the items, the meters, then what the PC knows and its notes.
    fn draw_place(&self, frame: &mut Frame, area: Rect, view: Option<&protocol::View>) {
        let Some(view) = view else {
            frame.render_widget(self.pane_block("Here", false), area);
            return;
        };
        let mut lines: Vec<Line> = Vec::new();
        for line in [&view.cant_act, &view.busy] {
            if !line.is_empty() {
                lines.push(Line::styled(line.clone(), self.style(ASH)));
            }
        }
        let mut here_spans = vec![self.span("Here: ", PARCHMENT)];
        for (index, (id, name)) in text::actor_ids_and_names(view).enumerate() {
            if index > 0 {
                here_spans.push(self.span(", ", PARCHMENT));
            }
            here_spans.push(if *id == view.viewer {
                self.span("You", AMBER)
            } else {
                self.span(name.clone(), TANGERINE)
            });
        }
        lines.push(Line::from(here_spans));
        self.push_heading(&mut lines, "Exits");
        for exit in &view.exits {
            let color = if exit.locked { ROSE } else { ORANGE };
            lines.push(Line::styled(text::exit_phrase(exit), self.style(color)));
        }
        if view.exits.is_empty() {
            lines.push(Line::styled("none", self.style(ASH)));
        }
        if !view.items.is_empty() {
            self.push_heading(&mut lines, "Items");
            for item in &view.items {
                lines.push(Line::styled(text::item_line(item), self.style(PARCHMENT)));
            }
        }
        for (heading, meters) in [("Meters", &view.meters), ("The world", &view.world_meters)] {
            if !meters.is_empty() {
                self.push_heading(&mut lines, heading);
                lines.extend(meters.iter().map(|meter| self.render_meter(meter)));
            }
        }
        let known_facts: Vec<&str> = view
            .known_facts
            .iter()
            .map(|fact| fact.text.as_str())
            .collect();
        let notes: Vec<&str> = view.notes.iter().map(|note| note.text.as_str()).collect();
        for (heading, sentences) in [("You know", known_facts), ("Notes", notes)] {
            if !sentences.is_empty() {
                self.push_heading(&mut lines, heading);
                for sentence in sentences {
                    lines.push(Line::styled(sentence.to_string(), self.style(PARCHMENT)));
                }
            }
        }
        let panel = Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(self.pane_block(&view.place_name, false));
        frame.render_widget(panel, area);
    }

    fn push_heading(&self, lines: &mut Vec<Line<'static>>, heading: &str) {
        lines.push(Line::default());
        lines.push(Line::styled(heading.to_string(), self.style(ASH)));
    }

    /// The meter as a line: `Hp: 8/16` and a gauge of its fill, scarlet on drip.
    fn render_meter(&self, meter: &protocol::MeterView) -> Line<'static> {
        let full_cells = match usize::try_from(meter.max) {
            Ok(max) if max > 0 => {
                let value = usize::try_from(meter.value.clamp(0, meter.max)).unwrap_or(0);
                (value * GAUGE_CELLS + max / 2) / max
            }
            _ => 0,
        };
        Line::from(vec![
            self.span(format!("{} ", text::meter_line(meter)), PARCHMENT),
            self.span("━".repeat(full_cells), SCARLET),
            self.span("─".repeat(GAUGE_CELLS - full_cells), DRIP),
        ])
    }

    /// The input line behind its prompt, with the cursor where the player types.
    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(DRIP));
        let [prompt_area, value_area] =
            Layout::horizontal([Constraint::Length(2), Constraint::Fill(1)])
                .areas(block.inner(area));
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(self.span("› ", FLAME)), prompt_area);
        let scroll_cells = self
            .input
            .visual_scroll(usize::from(value_area.width.saturating_sub(1)));
        let value = self.span(self.input.value().to_string(), PARCHMENT);
        let scroll_x = u16::try_from(scroll_cells).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(value).scroll((0, scroll_x)), value_area);
        let cursor_x = u16::try_from(self.input.visual_cursor() - scroll_cells).unwrap_or(u16::MAX);
        frame.set_cursor_position((value_area.x.saturating_add(cursor_x), value_area.y));
    }

    /// The key bar: each key's name in tangerine and its action in ash.
    fn draw_key_bar(&self, frame: &mut Frame, area: Rect, key_hints: &[(&str, &str)]) {
        let mut spans = Vec::new();
        for (key, action) in key_hints {
            spans.push(self.span(format!(" {key} "), TANGERINE));
            spans.push(self.span(format!("{action} "), ASH));
        }
        let dusk = DUSK.to_color(self.is_truecolor);
        frame.render_widget(Paragraph::new(Line::from(spans)).bg(dusk), area);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::Value;

    use super::{KeyRequest, Screen};
    use crate::protocol::{self, ServerBody, ServerMessage};

    const RECORDING: &str = include_str!("../../protocol/sessions/v2.jsonl");

    fn press(screen: &mut Screen, code: KeyCode, commands: &[String]) -> Option<KeyRequest> {
        screen.on_key(KeyEvent::from(code), commands)
    }

    fn type_text(screen: &mut Screen, text: &str, commands: &[String]) {
        for letter in text.chars() {
            press(screen, KeyCode::Char(letter), commands);
        }
    }

    /// Replays Rowan's side of the recorded docks session: shows each message Rowan got, picks
    /// Rowan at the picker, types each command Rowan sent, and stops before the first command
    /// named `stop`. Returns the latest view and the hello command list.
    fn replay_until(screen: &mut Screen, stop: &str) -> (Option<protocol::View>, Vec<String>) {
        let mut view = None;
        let mut commands = Vec::new();
        for row in RECORDING.lines() {
            let row: Value = serde_json::from_str(row).unwrap();
            if row["seat"] != "player:rowan" {
                continue;
            }
            if let Some(sent) = row.get("send") {
                let (command, args) = (&sent["body"]["kind"], &sent["body"]["args"]);
                let typed = match command.as_str().unwrap_or_default() {
                    "claim" => continue,
                    command if command == stop => return (view, commands),
                    "say" => format!("say {}", args["text"].as_str().unwrap_or_default()),
                    command => format!("{command} {}", args["target"].as_str().unwrap_or_default()),
                };
                type_text(screen, typed.trim(), &commands);
                press(screen, KeyCode::Enter, &commands);
                continue;
            }
            let message: ServerMessage = serde_json::from_value(row["get"].clone()).unwrap();
            if let ServerBody::Hello {
                commands: listed, ..
            } = &message.body
            {
                commands = listed.clone();
            }
            if let ServerBody::View(body) = &message.body {
                view = Some((**body).clone());
            }
            screen.show(&message.body);
            if matches!(message.body, ServerBody::Hello { .. }) {
                press(screen, KeyCode::Down, &commands);
                assert_eq!(
                    press(screen, KeyCode::Enter, &commands),
                    Some(KeyRequest::Send("rowan".into()))
                );
            }
        }
        (view, commands)
    }

    /// The screen's text at `width` by `height`, one row per line.
    fn screen_text(
        screen: &mut Screen,
        view: Option<&protocol::View>,
        width: u16,
        height: u16,
    ) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| screen.draw(frame, view)).unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..height)
            .map(|y| {
                let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect();
        rows.join("\n")
    }

    fn assert_screen(name: &str, screen: &mut Screen, view: Option<&protocol::View>) {
        let shown = screen_text(screen, view, 80, 24);
        insta::assert_snapshot!(format!("{name}_80x24"), shown);
    }

    #[test]
    fn the_picker_lists_each_pc_by_name() {
        let mut screen = Screen::new(true);
        let hello = RECORDING
            .lines()
            .find(|row| row.contains(r#""kind":"hello","version":2,"commands":["claim""#))
            .unwrap();
        let hello: Value = serde_json::from_str(hello).unwrap();
        let hello: ServerMessage = serde_json::from_value(hello["get"].clone()).unwrap();
        screen.show(&hello.body);
        assert_screen("picker", &mut screen, None);
    }

    #[test]
    fn a_claimed_pc_shows_the_transcript_and_the_place() {
        let mut screen = Screen::new(true);
        let (view, _) = replay_until(&mut screen, "travel");
        assert_screen("claimed", &mut screen, view.as_ref());
    }

    #[test]
    fn tab_completes_a_command_name() {
        let mut screen = Screen::new(true);
        let (view, commands) = replay_until(&mut screen, "travel");
        type_text(&mut screen, "tr", &commands);
        press(&mut screen, KeyCode::Tab, &commands);
        type_text(&mut screen, "stairs", &commands);
        assert_screen("completed", &mut screen, view.as_ref());
    }

    #[test]
    fn page_up_scrolls_the_transcript_back() {
        let mut screen = Screen::new(true);
        let (view, commands) = replay_until(&mut screen, "release");
        screen_text(&mut screen, view.as_ref(), 80, 24);
        press(&mut screen, KeyCode::PageUp, &commands);
        assert_screen("scrolled_back", &mut screen, view.as_ref());
    }

    #[test]
    fn a_refusal_shows_in_the_transcript() {
        let mut screen = Screen::new(true);
        let (view, _) = replay_until(&mut screen, "release");
        assert_screen("refused", &mut screen, view.as_ref());
    }
}
