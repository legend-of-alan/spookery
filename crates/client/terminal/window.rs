//! The game window `spook play` opens at a terminal: the PC picker, then the transcript, the
//! place panel, the tab box, the input line and the hotkey bar, painted in the Spookery palette.
//! `Terminal` hands it each message body and reads from it each line the player sends.

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::symbols;
use ratatui::text::{Line, Span};
use ratatui::widgets::{
    Block, BorderType, Clear, LineGauge, List, ListItem, ListState, Paragraph, Tabs, Wrap,
};
use ratatui::{DefaultTerminal, Frame};
use tui_input::backend::crossterm::EventHandler;
use tui_input::Input;

use crate::protocol::{self, Outcome, ServerBody, ServerMessage, Speaker, Speech};
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
const MULBERRY: PaletteColor = PaletteColor::new(0xa21d85, 126);
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

/// The terminal width from which the place panel and the tab box stand side by side.
const WIDE_LAYOUT_COLUMNS: u16 = 120;

/// How many columns the place panel and the tab box each take.
const SIDE_PANEL_COLUMNS: u16 = 34;

/// The meter whose gauge fills in scarlet. Every other meter's fills in gold.
const HP_METER_ID: &str = "hp";

/// What the tab box shows: the PC's meters, what it carries, or what it knows.
#[derive(Clone, Copy, PartialEq)]
enum Tab {
    Stats,
    Carried,
    Known,
}

impl Tab {
    /// Every tab in the order the tab row shows it, which is its key's order: F2, F3, F4.
    const ALL: [Tab; 3] = [Tab::Stats, Tab::Carried, Tab::Known];

    fn title(self) -> &'static str {
        match self {
            Tab::Stats => "Stats",
            Tab::Carried => "Carried",
            Tab::Known => "Known",
        }
    }
}

/// One transcript line, and whether it is a round divider, which the compact layout leaves
/// out.
struct TranscriptLine {
    line: Line<'static>,
    is_round_divider: bool,
}

impl From<Line<'static>> for TranscriptLine {
    fn from(line: Line<'static>) -> Self {
        Self {
            line,
            is_round_divider: false,
        }
    }
}

/// Where each part of the screen goes at one terminal size. `ScreenAreas::new` is the one place
/// that picks it.
struct ScreenAreas {
    title_rect: Rect,
    rule_rect: Rect,
    /// The rows between the rule and the input, where the PC picker goes.
    middle_rect: Rect,
    transcript_rect: Rect,
    place_panel_rect: Rect,
    /// `None` while the player has the tab box hidden.
    tab_box_rect: Option<Rect>,
    input_rect: Rect,
    hotkey_bar_rect: Rect,
    /// From `WIDE_LAYOUT_COLUMNS`, where the transcript shows round dividers.
    is_wide: bool,
}

impl ScreenAreas {
    /// The compact layout below `WIDE_LAYOUT_COLUMNS`: the transcript, and a right column with
    /// the place panel above the tab box, which gets `tab_box_content_rows` and its borders.
    /// The wide layout: the transcript, the place panel and the tab box side by side.
    /// `tab_box_content_rows` is `None` while the tab box is hidden, which gives its rows to the
    /// place panel, or in the wide layout its columns to the transcript.
    fn new(area: Rect, tab_box_content_rows: Option<u16>) -> Self {
        let [title_rect, rule_rect, middle_rect, input_rect, hotkey_bar_rect] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(area);
        let is_wide = area.width >= WIDE_LAYOUT_COLUMNS;
        let side_column = Constraint::Length(SIDE_PANEL_COLUMNS);
        let (transcript_rect, place_panel_rect, tab_box_rect) =
            match (tab_box_content_rows, is_wide) {
                (Some(_), true) => {
                    let [transcript_rect, place_panel_rect, tab_box_rect] =
                        Layout::horizontal([Constraint::Fill(1), side_column, side_column])
                            .areas(middle_rect);
                    (transcript_rect, place_panel_rect, Some(tab_box_rect))
                }
                (Some(content_rows), false) => {
                    let [transcript_rect, right_column] =
                        Layout::horizontal([Constraint::Fill(1), side_column]).areas(middle_rect);
                    let [place_panel_rect, tab_box_rect] = Layout::vertical([
                        Constraint::Fill(1),
                        Constraint::Length(content_rows + 2),
                    ])
                    .areas(right_column);
                    (transcript_rect, place_panel_rect, Some(tab_box_rect))
                }
                (None, _) => {
                    let [transcript_rect, place_panel_rect] =
                        Layout::horizontal([Constraint::Fill(1), side_column]).areas(middle_rect);
                    (transcript_rect, place_panel_rect, None)
                }
            };
        Self {
            title_rect,
            rule_rect,
            middle_rect,
            transcript_rect,
            place_panel_rect,
            tab_box_rect,
            input_rect,
            hotkey_bar_rect,
            is_wide,
        }
    }
}

/// How many rows the Stats tab needs: one per meter, then `The world` and one per world meter.
/// Every tab gets this many in the compact layout, so switching tabs keeps the tab box's size.
fn stats_rows(view: Option<&protocol::View>) -> u16 {
    let rows = view.map_or(0, |view| {
        let world_rows = match view.world_meters.len() {
            0 => 0,
            world_meter_count => world_meter_count + 1,
        };
        view.meters.len() + world_rows
    });
    u16::try_from(rows.max(1)).unwrap_or(u16::MAX)
}

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

    /// Adds one message to the screen and draws it.
    pub(crate) fn show(
        &mut self,
        message: &ServerMessage,
        view: Option<&protocol::View>,
    ) -> Result<(), Error> {
        self.screen.show(message);
        self.draw(view)
    }

    /// Takes the request id play sent the last line under.
    pub(crate) fn sent(&mut self, request_id: String) {
        self.screen.sent(request_id);
    }

    /// Handles keys until the player sends a line: a PC id at the PC picker, else a typed line,
    /// or `help` for F1. `map` at once when the PC has just arrived at a new place. `None` when
    /// they quit with Ctrl+C.
    pub(crate) async fn next_line(
        &mut self,
        view: Option<&protocol::View>,
        commands: &[String],
    ) -> Result<Option<String>, Error> {
        if let Some(line) = self.screen.take_own_line() {
            return Ok(Some(line));
        }
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
/// place panel, tab box and input line, and the help popup over them.
struct Screen {
    is_truecolor: bool,
    pc_picker: Option<PcPicker>,
    /// The last round's index for the title bar, `None` before the first round.
    last_round_index: Option<u64>,
    /// The world time after the last round.
    world_time_ticks: u64,
    renderer: text::Renderer,
    /// Every line the window has shown in the transcript pane, oldest first.
    transcript: Vec<TranscriptLine>,
    /// The tab the tab box shows, `None` while the player has it hidden.
    open_tab: Option<Tab>,
    /// The last `help` reply as text, shown in a popup until Esc.
    help_text: Option<String>,
    /// Whether the PC has arrived somewhere since the window last sent its own `map`.
    is_map_due: bool,
    /// Whether the last line the window handed play is its own `map`, until play says which
    /// request id it went under.
    is_own_map_unsent: bool,
    /// The request ids of the window's own `map` requests that have no reply yet.
    own_map_request_ids: Vec<String>,
    /// The places of the last `map` reply.
    place_views: Vec<protocol::PlaceView>,
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
            open_tab: Some(Tab::Stats),
            help_text: None,
            is_map_due: false,
            is_own_map_unsent: false,
            own_map_request_ids: Vec::new(),
            place_views: Vec::new(),
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
            .map(|line| Line::styled(line.to_string(), style).into());
        self.transcript.extend(lines);
    }

    fn push_line(&mut self, line: Line<'static>) {
        self.transcript.push(line.into());
    }

    /// The line the window sends without a key: `map` once after each arrival, so the place
    /// panel can list what lies past the exits.
    fn take_own_line(&mut self) -> Option<String> {
        self.is_own_map_unsent = std::mem::take(&mut self.is_map_due);
        self.is_own_map_unsent.then(|| "map".into())
    }

    /// Takes the request id play sent the last line under, which marks the reply to the
    /// window's own `map`.
    fn sent(&mut self, request_id: String) {
        if std::mem::take(&mut self.is_own_map_unsent) {
            self.own_map_request_ids.push(request_id);
        }
    }

    /// Adds what the message shows to the transcript, after the scrollback separator when the
    /// renderer says one is due. A reply to the window's own `map` only updates the place panel.
    fn show(&mut self, message: &ServerMessage) {
        let body = &message.body;
        if let ServerBody::Map { places } = body {
            self.place_views = places.clone();
        }
        let own_reply_index = (self.own_map_request_ids.iter())
            .position(|request_id| message.reply_to.as_ref() == Some(request_id));
        if let Some(index) = own_reply_index {
            self.own_map_request_ids.remove(index);
            return;
        }
        let first_new_line = self.transcript.len();
        self.push_body(body);
        let has_text = self.transcript.len() > first_new_line;
        if self.renderer.take_scrollback_separator(body, has_text) {
            self.transcript
                .insert(first_new_line, Line::default().into());
        }
    }

    fn push_body(&mut self, body: &ServerBody) {
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
                self.transcript.push(TranscriptLine {
                    line: Line::styled(divider, self.style(MULBERRY)),
                    is_round_divider: true,
                });
            }
            ServerBody::View(view) => {
                if view.place != self.last_place_id {
                    self.last_place_id = view.place.clone();
                    self.is_map_due = true;
                    self.push_place_arrival(view);
                }
            }
            ServerBody::Say(speech) => {
                let line = self.render_speech(speech);
                self.push_line(line);
            }
            ServerBody::Check { outcome } => {
                let color = match outcome {
                    Outcome::Success => GOLD,
                    Outcome::Partial => EMBER,
                    Outcome::Miss => SCARLET,
                };
                let sentence = self.renderer.render_without_separator(body);
                self.push_line(Line::styled(format!("◆ {sentence}"), self.style(color)));
            }
            ServerBody::Refused(refusal) => {
                self.push_text(&text::render_refusal(refusal, false), ROSE);
            }
            ServerBody::Changes { .. } | ServerBody::Queued { .. } => {
                let content = self.renderer.render_without_separator(body);
                self.push_text(&content, ASH);
            }
            ServerBody::Help { commands } => self.help_text = Some(text::render_help(commands)),
            ServerBody::Map { places } => self.push_text(&text::render_map(places), PARCHMENT),
            ServerBody::Claimed { .. } | ServerBody::Resume { .. } | ServerBody::Lines { .. } => {
                let content = self.renderer.render_without_separator(body);
                self.push_text(&content, PARCHMENT);
            }
            ServerBody::Released
            | ServerBody::Presence { .. }
            | ServerBody::Committed(_)
            | ServerBody::Status(_) => {}
        }
    }

    /// The place's name and prose, shown once when the PC arrives.
    fn push_place_arrival(&mut self, view: &protocol::View) {
        if !view.place_name.is_empty() {
            let name = self.span(view.place_name.clone(), GOLD);
            let marker = self.span("▌ ", FLAME);
            self.push_line(Line::from(vec![marker, name.add_modifier(Modifier::BOLD)]));
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
            KeyCode::F(1) => return Some(KeyRequest::Send("help".into())),
            KeyCode::F(function_key_number @ 2..=4) => {
                let tab = Tab::ALL[usize::from(function_key_number - 2)];
                self.open_tab = (self.open_tab != Some(tab)).then_some(tab);
            }
            KeyCode::Esc => self.help_text = None,
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
        self.push_line(echo);
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

    /// Where each part of the screen goes in `area` with the tab box as it stands.
    fn areas(&self, area: Rect, view: Option<&protocol::View>) -> ScreenAreas {
        ScreenAreas::new(area, self.open_tab.map(|_| stats_rows(view)))
    }

    fn draw(&mut self, frame: &mut Frame, view: Option<&protocol::View>) {
        let area = frame.area();
        let void = VOID.to_color(self.is_truecolor);
        frame.render_widget(Block::new().style(Style::new().bg(void)), area);
        let areas = self.areas(area, view);
        self.draw_title(frame, areas.title_rect, view);
        self.draw_rule(frame, areas.rule_rect);
        if self.pc_picker.is_some() {
            self.draw_pc_picker(frame, areas.middle_rect);
            self.draw_hotkey_bar(
                frame,
                areas.hotkey_bar_rect,
                &[("↑↓", "choose"), ("Enter", "play"), ("Ctrl+C", "quit")],
            );
            return;
        }
        self.draw_transcript(frame, areas.transcript_rect, areas.is_wide);
        self.draw_place(frame, areas.place_panel_rect, view);
        if let (Some(tab_box_area), Some(tab)) = (areas.tab_box_rect, self.open_tab) {
            self.draw_tab_box(frame, tab_box_area, tab, view);
        }
        self.draw_input(frame, areas.input_rect);
        if let Some(help_text) = &self.help_text {
            self.draw_help_popup(frame, areas.middle_rect, help_text);
        }
        self.draw_hotkey_bar(
            frame,
            areas.hotkey_bar_rect,
            &[
                ("Enter", "send"),
                ("Tab", "complete"),
                ("↑↓", "history"),
                ("PgUp PgDn", "scroll"),
                ("F1", "help"),
                ("Ctrl+C", "quit"),
            ],
        );
    }

    /// A pane's rounded border with `title` on it: plum with an amber title when
    /// `is_highlighted`, else mulberry with a tangerine title.
    fn titled_border(&self, title: &str, is_highlighted: bool) -> Block<'static> {
        let (border_color, title_color) = if is_highlighted {
            (PLUM, AMBER)
        } else {
            (MULBERRY, TANGERINE)
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

    /// A plum rule of upper half blocks with mulberry drops hanging from it.
    fn draw_rule(&self, frame: &mut Frame, area: Rect) {
        let rule_color = PLUM.to_color(self.is_truecolor);
        let drop_color = MULBERRY.to_color(self.is_truecolor);
        let gap_color = VOID.to_color(self.is_truecolor);
        let rule_spans: Vec<Span> = (0..area.width)
            .map(|column| {
                let background = if (column * 5 + 3) % 13 < 2 {
                    drop_color
                } else {
                    gap_color
                };
                Span::styled("▀", Style::new().fg(rule_color).bg(background))
            })
            .collect();
        frame.render_widget(Paragraph::new(Line::from(rule_spans)), area);
    }

    fn draw_pc_picker(&mut self, frame: &mut Frame, area: Rect) {
        let border = self.titled_border(text::PC_PICKER_HEADING, true);
        let highlight_style = self.style(GOLD).add_modifier(Modifier::BOLD);
        let (open_style, taken_style) = (self.style(PARCHMENT), self.style(ASH));
        let Some(pc_picker) = &mut self.pc_picker else {
            return;
        };
        let mut pc_list_items: Vec<ListItem> = pc_picker
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
            pc_list_items.push(ListItem::new(text::PC_PICKER_NONE_OPEN).style(taken_style));
        }
        let pc_list = List::new(pc_list_items)
            .block(border)
            .highlight_style(highlight_style)
            .highlight_symbol("› ");
        frame.render_stateful_widget(pc_list, area, &mut pc_picker.list_state);
    }

    /// The transcript, wrapped and pinned to the bottom unless scrolled back.
    fn draw_transcript(&mut self, frame: &mut Frame, area: Rect, is_showing_round_dividers: bool) {
        let border = self.titled_border("Transcript", true);
        let inner_area = border.inner(area);
        let lines: Vec<Line> = (self.transcript.iter())
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

    /// Who is here, the exits, the places past them that the map knows, and the items here.
    /// A section is left out, with all after it, when its heading and first row don't both fit.
    /// While the tab box is hidden, the bottom border names the keys that bring it back.
    fn draw_place(&self, frame: &mut Frame, area: Rect, view: Option<&protocol::View>) {
        let title = view.map_or("Here", |view| view.place_name.as_str());
        let mut border = self.titled_border(title, false);
        if self.open_tab.is_none() {
            border = border.title_bottom(self.span("F2 F3 F4 tabs", TANGERINE));
        }
        let Some(view) = view else {
            frame.render_widget(border, area);
            return;
        };
        let mut here_lines: Vec<Line> = Vec::new();
        for line in [&view.cant_act, &view.busy] {
            if !line.is_empty() {
                here_lines.push(Line::styled(line.clone(), self.style(ASH)));
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
        here_lines.push(Line::from(here_spans));
        let mut exit_lines = vec![Line::from(self.span("Exits", ASH))];
        for exit in &view.exits {
            let color = if exit.locked { ROSE } else { ORANGE };
            exit_lines.push(Line::styled(text::render_exit(exit), self.style(color)));
        }
        if view.exits.is_empty() {
            exit_lines.push(Line::styled("none", self.style(ASH)));
        }
        let mut sections = vec![here_lines, exit_lines];
        let nearby_lines: Vec<Line> = (view.exits.iter())
            .filter_map(|exit| {
                let place =
                    (self.place_views.iter()).find(|place| place.place_id == exit.place_id)?;
                Some(self.render_nearby_place(place, &view.place))
            })
            .collect();
        if !nearby_lines.is_empty() {
            let heading = Line::from(self.span("Nearby", ASH));
            sections.push(std::iter::once(heading).chain(nearby_lines).collect());
        }
        if !view.items.is_empty() {
            let heading = Line::from(self.span("Items here", ASH));
            let item_lines = (view.items.iter())
                .map(|item| Line::styled(text::render_item(item), self.style(PARCHMENT)));
            sections.push(std::iter::once(heading).chain(item_lines).collect());
        }
        let inner_width = border.inner(area).width;
        let line_rows = |line: &Line<'static>| {
            Paragraph::new(line.clone())
                .wrap(Wrap { trim: true })
                .line_count(inner_width)
        };
        let mut rows_left = usize::from(border.inner(area).height);
        let mut panel_lines: Vec<Line> = Vec::new();
        for section in sections {
            if section.iter().take(2).map(line_rows).sum::<usize>() > rows_left {
                break;
            }
            let section_rows: usize = section.iter().map(line_rows).sum();
            rows_left = rows_left.saturating_sub(section_rows);
            panel_lines.extend(section);
        }
        let panel = Paragraph::new(panel_lines)
            .wrap(Wrap { trim: true })
            .block(border);
        frame.render_widget(panel, area);
    }

    /// `Docks Street → Gate Yard`: a place an exit leads to, then where its exits go from there
    /// as the map last saw them, leaving out the way back to `here_place_id`.
    fn render_nearby_place(
        &self,
        place: &protocol::PlaceView,
        here_place_id: &str,
    ) -> Line<'static> {
        let onward_place_names: Vec<&str> = (place.exits.iter())
            .filter(|exit| exit.place_id != here_place_id)
            .map(|exit| exit.place_name.as_str())
            .collect();
        let mut spans = vec![self.span(place.name.clone(), GOLD)];
        if !onward_place_names.is_empty() {
            let onward = format!(" → {}", onward_place_names.join(", "));
            spans.push(self.span(onward, PARCHMENT));
        }
        Line::from(spans)
    }

    /// The tab row on the top border, `tab`'s content under it, and the tab keys on the bottom
    /// border.
    fn draw_tab_box(&self, frame: &mut Frame, area: Rect, tab: Tab, view: Option<&protocol::View>) {
        let border = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(MULBERRY))
            .title_bottom(self.span("F2 F3 F4 switch · same key hides", TANGERINE));
        let content_area = border.inner(area);
        let tabs_area = Rect {
            y: area.y,
            height: 1,
            ..content_area
        };
        frame.render_widget(border, area);
        let tab_titles = Tab::ALL.map(|listed_tab| self.span(listed_tab.title(), TANGERINE));
        let tabs = Tabs::new(tab_titles)
            .select(tab as usize)
            .highlight_style(self.style(AMBER).add_modifier(Modifier::BOLD))
            .divider(self.span("│", MULBERRY));
        frame.render_widget(tabs, tabs_area);
        let Some(view) = view else {
            return;
        };
        match tab {
            Tab::Stats => self.draw_meters(frame, content_area, view),
            Tab::Carried => self.draw_carried_items(frame, content_area, view),
            Tab::Known => self.draw_facts_and_notes(frame, content_area, view),
        }
    }

    /// Each meter's gauge, then `The world` and the world's meters.
    fn draw_meters(&self, frame: &mut Frame, area: Rect, view: &protocol::View) {
        let label_width = (view.meters.iter().chain(&view.world_meters))
            .map(|meter| text::render_meter(meter).chars().count())
            .max()
            .unwrap_or(0);
        let mut rows = area.rows();
        for (heading, meters) in [
            (None, &view.meters),
            (Some("The world"), &view.world_meters),
        ] {
            if meters.is_empty() {
                continue;
            }
            if let Some(heading) = heading {
                if let Some(row) = rows.next() {
                    frame.render_widget(Paragraph::new(self.span(heading, ASH)), row);
                }
            }
            for (meter, row) in meters.iter().zip(&mut rows) {
                frame.render_widget(self.render_meter_gauge(meter, label_width), row);
            }
        }
    }

    /// `Health: 8/16` padded to `label_width`, then the meter's fill: scarlet for Hp, gold for
    /// any other, on mulberry.
    fn render_meter_gauge(
        &self,
        meter: &protocol::MeterView,
        label_width: usize,
    ) -> LineGauge<'static> {
        let ratio = if meter.max > 0 {
            meter.value.clamp(0, meter.max) as f64 / meter.max as f64
        } else {
            0.0
        };
        let fill_color = if meter.id == HP_METER_ID {
            SCARLET
        } else {
            GOLD
        };
        let label = format!("{:label_width$}", text::render_meter(meter));
        LineGauge::default()
            .ratio(ratio)
            .label(self.span(label, PARCHMENT))
            .filled_symbol(symbols::line::THICK_HORIZONTAL)
            .filled_style(self.style(fill_color))
            .unfilled_style(self.style(MULBERRY))
    }

    /// Each item the PC carries, the wielded one in gold.
    fn draw_carried_items(&self, frame: &mut Frame, area: Rect, view: &protocol::View) {
        let mut item_list_items: Vec<ListItem> = (view.items_carried.iter())
            .map(|item| {
                let is_wielded = view.wielded_item_id.as_ref() == Some(&item.id);
                let color = if is_wielded { GOLD } else { PARCHMENT };
                let line = text::render_carried_item(item, is_wielded);
                ListItem::new(self.span(line, color))
            })
            .collect();
        if item_list_items.is_empty() {
            item_list_items.push(ListItem::new(self.span("Nothing", ASH)));
        }
        frame.render_widget(List::new(item_list_items), area);
    }

    /// The facts the PC knows and its notes, worded as the line client words them.
    fn draw_facts_and_notes(&self, frame: &mut Frame, area: Rect, view: &protocol::View) {
        let sections = text::render_facts_and_notes(&view.known_facts, &view.notes);
        let known_text = if sections.is_empty() {
            Paragraph::new(self.span("Nothing yet", ASH))
        } else {
            Paragraph::new(sections.join("\n\n")).style(self.style(PARCHMENT))
        };
        frame.render_widget(known_text.wrap(Wrap { trim: true }), area);
    }

    /// The `help` reply over the middle of `area`, on dusk, until Esc.
    fn draw_help_popup(&self, frame: &mut Frame, area: Rect, help_text: &str) {
        let border = self
            .titled_border("Help", true)
            .title_bottom(self.span(" Esc closes ", TANGERINE));
        let help = Paragraph::new(help_text.to_string())
            .style(self.style(PARCHMENT))
            .bg(DUSK.to_color(self.is_truecolor))
            .wrap(Wrap { trim: false });
        let width = area.width.min(60);
        let wrapped_rows = help.line_count(width.saturating_sub(2)) + 2;
        let height = u16::try_from(wrapped_rows).unwrap_or(u16::MAX);
        let popup_area = area.centered(Constraint::Length(width), Constraint::Max(height));
        let help = help.block(border);
        frame.render_widget(Clear, popup_area);
        frame.render_widget(help, popup_area);
    }

    /// The input line behind its prompt, with the cursor where the player types.
    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        let border = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(MULBERRY));
        let [prompt_area, value_area] =
            Layout::horizontal([Constraint::Length(2), Constraint::Fill(1)])
                .areas(border.inner(area));
        frame.render_widget(border, area);
        frame.render_widget(Paragraph::new(self.span("› ", FLAME)), prompt_area);
        let scroll_cells = self
            .input
            .visual_scroll(usize::from(value_area.width.saturating_sub(1)));
        let input_span = self.span(self.input.value().to_string(), PARCHMENT);
        let scroll_x = u16::try_from(scroll_cells).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(input_span).scroll((0, scroll_x)), value_area);
        let cursor_x = u16::try_from(self.input.visual_cursor() - scroll_cells).unwrap_or(u16::MAX);
        frame.set_cursor_position((value_area.x.saturating_add(cursor_x), value_area.y));
    }

    /// The hotkey bar: each key's name in tangerine and its action in ash.
    fn draw_hotkey_bar(&self, frame: &mut Frame, area: Rect, hotkey_hints: &[(&str, &str)]) {
        let mut hint_spans = Vec::new();
        for (key, action) in hotkey_hints {
            hint_spans.push(self.span(format!(" {key} "), TANGERINE));
            hint_spans.push(self.span(format!("{action} "), ASH));
        }
        let dusk = DUSK.to_color(self.is_truecolor);
        frame.render_widget(Paragraph::new(Line::from(hint_spans)).bg(dusk), area);
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::backend::TestBackend;
    use ratatui::buffer::Buffer;
    use ratatui::layout::Rect;
    use ratatui::Terminal;
    use serde_json::Value;

    use super::{KeyRequest, Screen, ScreenAreas};
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
            screen.show(&message);
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
        let buffer = draw(screen, view, width, height);
        text_inside(&buffer, buffer.area)
    }

    /// The screen's text at 80 by 24 inside the one part `pick_rect` takes from its areas.
    fn region_text(
        screen: &mut Screen,
        view: Option<&protocol::View>,
        pick_rect: fn(&ScreenAreas) -> Option<Rect>,
    ) -> String {
        let buffer = draw(screen, view, 80, 24);
        let rect = pick_rect(&screen.areas(buffer.area, view)).unwrap();
        text_inside(&buffer, rect)
    }

    fn draw(screen: &mut Screen, view: Option<&protocol::View>, width: u16, height: u16) -> Buffer {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| screen.draw(frame, view)).unwrap();
        terminal.backend().buffer().clone()
    }

    /// The text `buffer` holds inside `rect`, one row per line.
    fn text_inside(buffer: &Buffer, rect: Rect) -> String {
        let rows: Vec<String> = (rect.top()..rect.bottom())
            .map(|y| {
                let row: String = (rect.left()..rect.right())
                    .map(|x| buffer[(x, y)].symbol())
                    .collect();
                row.trim_end().to_string()
            })
            .collect();
        rows.join("\n")
    }

    fn assert_screen(name: &str, screen: &mut Screen, view: Option<&protocol::View>) {
        assert_screen_at(name, screen, view, 80, 24);
    }

    fn assert_screen_at(
        name: &str,
        screen: &mut Screen,
        view: Option<&protocol::View>,
        width: u16,
        height: u16,
    ) {
        let shown = screen_text(screen, view, width, height);
        insta::assert_snapshot!(format!("{name}_{width}x{height}"), shown);
    }

    /// Shows one message written as its JSON line, and returns its view when it is one.
    fn show_json(screen: &mut Screen, message: &str) -> Option<protocol::View> {
        let message: ServerMessage = serde_json::from_str(message).unwrap();
        screen.show(&message);
        match message.body {
            ServerBody::View(view) => Some(*view),
            _ => None,
        }
    }

    /// Rowan's view after taking a tin of lamp oil, from a docks session against this build,
    /// with `wielded_item_id` and a world meter added, since no docks item is a weapon and docks
    /// has no world meter.
    const VIEW_CARRYING_OIL: &str = r#"{"seq":24,"body":{"kind":"view","viewer":"rowan","place":"docks_shop","place_name":"Docks Shop","body":"The shop faces the water. Tins sit in a row.","actor_names":["Aldric","Rowan"],"visible_actors":["aldric","rowan"],"known_facts":[{"id":"fact_oil_on_shelf","text":"Oil is on the shelf.","owner_body":"The shop faces the water. Tins sit in a row.\n"}],"notes":[{"id":"note_mira_door","to_ids":["rowan"],"place_id":"docks_shop","fact_ids":[],"text":"Back at dusk. Oil on the shelf."}],"unknown_mentions":[],"items":[{"id":"lamp_oil","count":3,"name":"Lamp oil"}],"items_carried":[{"id":"lamp_oil","count":1,"name":"Lamp oil"}],"wielded_item_id":"lamp_oil","meters":[{"id":"assassination_debt","value":2,"max":6},{"id":"hp","name":"Health","value":8,"max":16},{"id":"might","value":1,"max":45},{"id":"rest","value":3,"max":5},{"id":"vigor","value":1,"max":110}],"world_meters":[{"id":"tide","name":"Tide","value":3,"max":10}],"habits":[],"exits":[{"exit_id":"docks_street","name":"Docks Street","place_id":"docks_street","place_name":"Docks Street","ticks":0,"locked":false},{"exit_id":"stairs","name":"stairs","place_id":"mira_rooms","place_name":"Mira's Rooms","ticks":2,"locked":true}]}}"#;

    /// Docks Street from Rowan's `map` reply in that session, after a walk there and back.
    const MAP_REPLY: &str = r#"{"seq":26,"body":{"kind":"map","places":[{"place_id":"docks_street","name":"Docks Street","exits":[{"exit_id":"docks_shop","name":"Docks Shop","place_id":"docks_shop","place_name":"Docks Shop","ticks":0,"locked":false},{"exit_id":"gate_yard","name":"Gate Yard","place_id":"gate_yard","place_name":"Gate Yard","ticks":0,"locked":false}]}]}}"#;

    /// Rowan's `help` reply in the Docks Shop after taking a tin of lamp oil, from another docks
    /// session against this build.
    const HELP_REPLY: &str = r#"{"seq":9,"reply_to":"c3","body":{"kind":"help","commands":[{"id":"release","target":"none"},{"id":"say","target":"none"},{"id":"look","target":"none"},{"id":"help","target":"none"},{"id":"map","target":"none"},{"id":"cancel","target":"none"},{"id":"drop","target":"item","names":["Lamp oil"]},{"id":"exert","target":"none"},{"id":"strike","target":"actor","names":["Aldric"]},{"id":"take","target":"item","names":["Lamp oil"]},{"id":"travel","target":"exit","names":["Docks Street","stairs"]},{"id":"unwield","target":"none"},{"id":"wait","target":"none"},{"id":"wield","target":"item","names":["Lamp oil"]}]}}"#;

    /// `MAP_REPLY` as the reply to `request_id`.
    fn map_reply(request_id: &str) -> ServerMessage {
        let mut reply: ServerMessage = serde_json::from_str(MAP_REPLY).unwrap();
        reply.reply_to = Some(request_id.into());
        reply
    }

    fn view_message(view: protocol::View) -> ServerMessage {
        ServerMessage {
            id: None,
            seq: None,
            reply_to: None,
            body: ServerBody::View(Box::new(view)),
        }
    }

    /// Takes the line the window sends on its own, which must be `map`, and sends it under
    /// `request_id`.
    fn send_own_map(screen: &mut Screen, request_id: &str) {
        assert_eq!(screen.take_own_line().as_deref(), Some("map"));
        screen.sent(request_id.into());
    }

    /// Rowan's recorded session up to `release`, then the view carrying lamp oil.
    fn seat_rowan_carrying_oil(screen: &mut Screen) -> (Option<protocol::View>, Vec<String>) {
        let (_, commands) = replay_until(screen, "release");
        (show_json(screen, VIEW_CARRYING_OIL), commands)
    }

    #[test]
    fn the_stats_tab_shows_each_meter_by_name_with_a_gauge() {
        let mut screen = Screen::new(true);
        let (view, _) = seat_rowan_carrying_oil(&mut screen);
        assert_screen("stats_tab", &mut screen, view.as_ref());
    }

    #[test]
    fn f3_shows_what_the_pc_carries_with_the_wielded_item_marked() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        press(&mut screen, KeyCode::F(3), &commands);
        assert_screen("carried_tab", &mut screen, view.as_ref());
    }

    #[test]
    fn f4_shows_the_facts_and_notes_the_pc_knows() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        press(&mut screen, KeyCode::F(4), &commands);
        assert_screen("known_tab", &mut screen, view.as_ref());
    }

    #[test]
    fn the_open_tabs_key_hides_the_tab_box() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        press(&mut screen, KeyCode::F(3), &commands);
        press(&mut screen, KeyCode::F(3), &commands);
        assert_screen("tab_box_hidden", &mut screen, view.as_ref());
    }

    #[test]
    fn f1_asks_for_help_and_esc_closes_its_popup() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        let before_help = screen_text(&mut screen, view.as_ref(), 80, 24);
        assert_eq!(
            press(&mut screen, KeyCode::F(1), &commands),
            Some(KeyRequest::Send("help".into()))
        );
        show_json(&mut screen, HELP_REPLY);
        assert_screen("help_popup", &mut screen, view.as_ref());
        press(&mut screen, KeyCode::Esc, &commands);
        assert_eq!(screen_text(&mut screen, view.as_ref(), 80, 24), before_help);
    }

    #[test]
    fn the_windows_own_map_reply_fills_nearby_and_stays_out_of_the_transcript() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        let view = view.unwrap();
        send_own_map(&mut screen, "c8");
        let street_view = protocol::View {
            place: "docks_street".into(),
            place_name: "Docks Street".into(),
            ..view.clone()
        };
        screen.show(&view_message(street_view));
        send_own_map(&mut screen, "c9");
        screen.show(&view_message(view.clone()));
        send_own_map(&mut screen, "c10");
        let transcript_rect = |areas: &ScreenAreas| Some(areas.transcript_rect);
        let transcript_before = region_text(&mut screen, Some(&view), transcript_rect);
        for request_id in ["c8", "c9", "c10"] {
            screen.show(&map_reply(request_id));
        }
        assert_eq!(
            region_text(&mut screen, Some(&view), transcript_rect),
            transcript_before
        );
        let place_panel_rect = |areas: &ScreenAreas| Some(areas.place_panel_rect);
        let place_panel = region_text(&mut screen, Some(&view), place_panel_rect);
        assert!(place_panel.contains("Docks Street → Gate Yard"));
        type_text(&mut screen, "map", &commands);
        assert_eq!(
            press(&mut screen, KeyCode::Enter, &commands),
            Some(KeyRequest::Send("map".into()))
        );
        assert_eq!(screen.take_own_line(), None);
        screen.sent("c11".into());
        screen.show(&map_reply("c11"));
        let transcript_after = region_text(&mut screen, Some(&view), transcript_rect);
        assert!(transcript_after.contains("Gate Yard — 0 ticks"));
    }

    #[test]
    fn the_place_panel_lists_nearby_places_from_the_map() {
        let mut screen = Screen::new(true);
        let (view, _) = seat_rowan_carrying_oil(&mut screen);
        send_own_map(&mut screen, "c8");
        screen.show(&map_reply("c8"));
        assert_screen("nearby", &mut screen, view.as_ref());
    }

    #[test]
    fn a_wide_terminal_shows_the_story_the_place_and_the_tab_box_side_by_side() {
        let mut screen = Screen::new(true);
        let (view, _) = seat_rowan_carrying_oil(&mut screen);
        send_own_map(&mut screen, "c8");
        screen.show(&map_reply("c8"));
        assert_screen_at("wide", &mut screen, view.as_ref(), 140, 40);
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
        screen.show(&hello);
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

    #[test]
    fn a_blank_line_follows_the_scrollback() {
        let mut screen = Screen::new(true);
        let messages = [
            r#"{"body":{"kind":"claimed","actor":"rowan","token":"t1","scrollback":[{"from":"gm","to":"table","text":"good evening"}]}}"#,
            r#"{"seq":3,"body":{"kind":"say","from":"player:rowan","to":"table","text":"evening"}}"#,
        ];
        for message in messages {
            let message: ServerMessage = serde_json::from_str(message).unwrap();
            screen.show(&message);
        }
        assert_screen("scrollback_separator", &mut screen, None);
    }
}
