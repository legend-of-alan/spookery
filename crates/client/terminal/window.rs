//! The game window `spook play` opens at a terminal: the PC picker, then the transcript, the
//! place panel, the tab box, the input line and the hotkey bar, painted in the Spookery palette.
//! `Terminal` hands it each message and reads from it each line the player sends.

mod input_line;
mod layout;
mod place_panel;
mod tab_box;
mod transcript;

use crossterm::event::{Event, EventStream, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use futures_util::StreamExt;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, List, ListItem, ListState, Paragraph, Wrap};
use ratatui::{DefaultTerminal, Frame};

use crate::protocol::{self, Outcome, ServerBody, ServerMessage, Speaker, Speech};
use crate::text;
use crate::Error;
use input_line::InputLine;
use layout::ScreenAreas;
use place_panel::PlacePanel;
use tab_box::{Tab, TabBox};
use transcript::Transcript;

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

/// How the window paints a palette color: as its truecolor value, or as its 256-color fallback.
#[derive(Clone, Copy)]
struct Palette {
    is_truecolor: bool,
}

impl Palette {
    fn color(self, color: PaletteColor) -> Color {
        if self.is_truecolor {
            let [_, r, g, b] = color.rgb.to_be_bytes();
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(color.ansi_index)
        }
    }

    fn style(self, color: PaletteColor) -> Style {
        Style::new().fg(self.color(color))
    }

    fn span(self, content: impl Into<String>, color: PaletteColor) -> Span<'static> {
        Span::styled(content.into(), self.style(color))
    }

    /// A rounded border with `title` on it: plum with an amber title when `is_highlighted`, else
    /// mulberry with a tangerine title.
    fn titled_border(self, title: &str, is_highlighted: bool) -> Block<'static> {
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
    pub(crate) fn record_request_id(&mut self, request_id: String) {
        self.screen.record_request_id(request_id);
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
    palette: Palette,
    pc_picker: Option<PcPicker>,
    /// The last round's index for the title bar, `None` before the first round.
    last_round_index: Option<u64>,
    /// The world time after the last round.
    world_time_ticks: u64,
    renderer: text::Renderer,
    transcript: Transcript,
    place_panel: PlacePanel,
    tab_box: TabBox,
    /// The last `help` reply as text, shown in a popup until Esc.
    help_text: Option<String>,
    /// The place whose name and prose the transcript showed last.
    last_place_id: String,
    input_line: InputLine,
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
            palette: Palette { is_truecolor },
            pc_picker: None,
            last_round_index: None,
            world_time_ticks: 0,
            renderer: text::Renderer::new(),
            transcript: Transcript::default(),
            place_panel: PlacePanel::default(),
            tab_box: TabBox::new(),
            help_text: None,
            last_place_id: String::new(),
            input_line: InputLine::default(),
        }
    }

    /// Adds each line of `content` to the transcript in `color`.
    fn push_text(&mut self, content: &str, color: PaletteColor) {
        for line in content.lines() {
            let line = Line::styled(line.to_string(), self.palette.style(color));
            self.transcript.push(line);
        }
    }

    /// The line the window sends without a key: `map` once after each arrival.
    fn take_own_line(&mut self) -> Option<String> {
        self.place_panel.take_map_line()
    }

    /// Takes the request id play sent the last line under.
    fn record_request_id(&mut self, request_id: String) {
        self.place_panel.record_request_id(request_id);
    }

    /// Adds what the message shows to the transcript, after the scrollback separator when the
    /// renderer says one is due. A reply to the window's own `map` only updates the place panel.
    fn show(&mut self, message: &ServerMessage) {
        let body = &message.body;
        self.place_panel.keep_map_places(body);
        if (self.place_panel).remove_own_map_request(message.reply_to.as_deref()) {
            return;
        }
        let first_new_line = self.transcript.line_count();
        self.push_body(body);
        let has_text = self.transcript.line_count() > first_new_line;
        if self.renderer.take_scrollback_separator(body, has_text) {
            self.transcript.insert_blank_line(first_new_line);
        }
    }

    fn push_body(&mut self, body: &ServerBody) {
        let palette = self.palette;
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
                let divider = Line::styled(divider, palette.style(MULBERRY));
                self.transcript.push_round_divider(divider);
            }
            ServerBody::View(view) => {
                if view.place != self.last_place_id {
                    self.last_place_id = view.place.clone();
                    self.place_panel.mark_map_due();
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
                let sentence = self.renderer.render_without_separator(body);
                let line = Line::styled(format!("◆ {sentence}"), palette.style(color));
                self.transcript.push(line);
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
        let palette = self.palette;
        if !view.place_name.is_empty() {
            let name = palette.span(view.place_name.clone(), GOLD);
            let marker = palette.span("▌ ", FLAME);
            let line = Line::from(vec![marker, name.add_modifier(Modifier::BOLD)]);
            self.transcript.push(line);
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
            self.palette
                .span(text::speaker_label(speech), speaker_color),
            self.palette.span(format!(": {}", speech.text), PARCHMENT),
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
                self.tab_box
                    .toggle(Tab::ALL[usize::from(function_key_number - 2)]);
            }
            KeyCode::Esc => self.help_text = None,
            KeyCode::Tab => self.input_line.complete_command(commands),
            KeyCode::Up | KeyCode::Down => self.input_line.recall_history(key.code == KeyCode::Up),
            KeyCode::PageUp => self.transcript.scroll_back(),
            KeyCode::PageDown => self.transcript.scroll_forward(),
            _ => self.input_line.handle_key(key),
        }
        None
    }

    /// Sends the input line, which the history keeps, and echoes it in the transcript.
    fn send_input(&mut self) -> Option<KeyRequest> {
        let line = self.input_line.take_line()?;
        let echo = Line::from(vec![
            self.palette.span("› ", FLAME),
            self.palette.span(line.clone(), BERRY),
        ]);
        self.transcript.push(echo);
        self.transcript.scroll_to_bottom();
        Some(KeyRequest::Send(line))
    }

    /// Where each part of the screen goes in `area` with the tab box as it stands.
    fn areas(&self, area: Rect, view: Option<&protocol::View>) -> ScreenAreas {
        ScreenAreas::new(area, self.tab_box.content_rows(view))
    }

    fn draw(&mut self, frame: &mut Frame, view: Option<&protocol::View>) {
        let palette = self.palette;
        let area = frame.area();
        frame.render_widget(
            Block::new().style(Style::new().bg(palette.color(VOID))),
            area,
        );
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
        self.transcript
            .draw(frame, areas.transcript_rect, areas.is_wide, palette);
        let is_tab_box_hidden = areas.tab_box_rect.is_none();
        (self.place_panel).draw(
            frame,
            areas.place_panel_rect,
            view,
            is_tab_box_hidden,
            palette,
        );
        if let Some(tab_box_rect) = areas.tab_box_rect {
            self.tab_box.draw(frame, tab_box_rect, view, palette);
        }
        self.input_line.draw(frame, areas.input_rect, palette);
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

    /// SPOOKERY in its gradient, then the PC's name from `view`, the last round and the tick.
    fn draw_title(&self, frame: &mut Frame, area: Rect, view: Option<&protocol::View>) {
        let palette = self.palette;
        let dusk = palette.color(DUSK);
        let mut letter_spans: Vec<Span> = vec![Span::raw(" ")];
        letter_spans.extend(TITLE_LETTERS.iter().map(|(letter, color)| {
            palette
                .span(letter.to_string(), *color)
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
        let pc_line =
            Line::styled(format!("{pc_label} "), palette.style(PARCHMENT)).right_aligned();
        frame.render_widget(Paragraph::new(pc_line).bg(dusk), pc_area);
    }

    /// A plum rule of upper half blocks with mulberry drops hanging from it.
    fn draw_rule(&self, frame: &mut Frame, area: Rect) {
        let rule_color = self.palette.color(PLUM);
        let drop_color = self.palette.color(MULBERRY);
        let gap_color = self.palette.color(VOID);
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
        let palette = self.palette;
        let border = palette.titled_border(text::PC_PICKER_HEADING, true);
        let highlight_style = palette.style(GOLD).add_modifier(Modifier::BOLD);
        let (open_style, taken_style) = (palette.style(PARCHMENT), palette.style(ASH));
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

    /// The `help` reply over the middle of `area`, on dusk, until Esc.
    fn draw_help_popup(&self, frame: &mut Frame, area: Rect, help_text: &str) {
        let palette = self.palette;
        let border = palette
            .titled_border("Help", true)
            .title_bottom(palette.span(" Esc closes ", TANGERINE));
        let help = Paragraph::new(help_text.to_string())
            .style(palette.style(PARCHMENT))
            .bg(palette.color(DUSK))
            .wrap(Wrap { trim: false });
        let width = area.width.min(60);
        let wrapped_rows = help.line_count(width.saturating_sub(2)) + 2;
        let height = u16::try_from(wrapped_rows).unwrap_or(u16::MAX);
        let popup_area = area.centered(Constraint::Length(width), Constraint::Max(height));
        let help = help.block(border);
        frame.render_widget(Clear, popup_area);
        frame.render_widget(help, popup_area);
    }

    /// The hotkey bar: each key's name in tangerine and its action in ash.
    fn draw_hotkey_bar(&self, frame: &mut Frame, area: Rect, hotkey_hints: &[(&str, &str)]) {
        let mut hint_spans = Vec::new();
        for (key, action) in hotkey_hints {
            hint_spans.push(self.palette.span(format!(" {key} "), TANGERINE));
            hint_spans.push(self.palette.span(format!("{action} "), ASH));
        }
        let dusk = self.palette.color(DUSK);
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

    const RECORDING: &str = include_str!("../../protocol/sessions/v3.jsonl");

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

    /// Compares `shown` to the snapshot named `name` in `crates/client/tests/snapshots`.
    fn assert_snapshot(name: &str, shown: &str) {
        insta::with_settings!({ snapshot_path => "../tests/snapshots" }, {
            insta::assert_snapshot!(name, shown);
        });
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
        assert_snapshot(&format!("{name}_{width}x{height}"), &shown);
    }

    /// Snapshots the one part of the 80 by 24 screen that `pick_rect` takes from its areas.
    fn assert_region(
        name: &str,
        screen: &mut Screen,
        view: &protocol::View,
        pick_rect: fn(&ScreenAreas) -> Option<Rect>,
    ) {
        assert_snapshot(name, &region_text(screen, Some(view), pick_rect));
    }

    /// Docks Street from Rowan's `map` reply in that session, after a walk there and back.
    const MAP_REPLY: &str = r#"{"seq":26,"body":{"kind":"map","places":[{"place_id":"docks_street","name":"Docks Street","exits":[{"exit_id":"docks_shop","name":"Docks Shop","place_id":"docks_shop","place_name":"Docks Shop","ticks":0,"locked":false},{"exit_id":"gate_yard","name":"Gate Yard","place_id":"gate_yard","place_name":"Gate Yard","ticks":0,"locked":false}]}]}}"#;

    /// Three commands from Rowan's `help` reply in the Docks Shop, from a docks session against
    /// this build.
    const HELP_REPLY: &str = r#"{"seq":9,"reply_to":"c3","body":{"kind":"help","commands":[{"id":"look","target":"none"},{"id":"strike","target":"actor","names":["Aldric"]},{"id":"travel","target":"exit","names":["Docks Street","stairs"]}]}}"#;

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
        screen.record_request_id(request_id.into());
    }

    /// Rowan's recorded session up to `release`, and Rowan's last view there as this build sends
    /// it: carrying and wielding a tin of lamp oil, with Hp named Health and one world meter,
    /// since no docks item is a weapon and docks has no world meter.
    fn seat_rowan_carrying_oil(screen: &mut Screen) -> (protocol::View, Vec<String>) {
        let (view, commands) = replay_until(screen, "release");
        let mut view = view.unwrap();
        let lamp_oil = protocol::ItemStack {
            id: "lamp_oil".into(),
            count: 1,
            name: "Lamp oil".into(),
        };
        view.items_carried = vec![lamp_oil];
        view.wielded_item_id = Some("lamp_oil".into());
        let hp = view
            .meters
            .iter_mut()
            .find(|meter| meter.id == "hp")
            .unwrap();
        hp.name = "Health".into();
        view.world_meters = vec![protocol::MeterView {
            id: "tide".into(),
            name: "Tide".into(),
            value: 3,
            max: 10,
        }];
        (view, commands)
    }

    #[test]
    fn the_stats_tab_shows_each_meter_by_name_with_a_gauge() {
        let mut screen = Screen::new(true);
        let (view, _) = seat_rowan_carrying_oil(&mut screen);
        assert_region("stats_tab", &mut screen, &view, |areas| areas.tab_box_rect);
    }

    #[test]
    fn f3_shows_what_the_pc_carries_with_the_wielded_item_marked() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        press(&mut screen, KeyCode::F(3), &commands);
        assert_region("carried_tab", &mut screen, &view, |areas| {
            areas.tab_box_rect
        });
    }

    #[test]
    fn f4_shows_the_notes_the_pc_reads() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        press(&mut screen, KeyCode::F(4), &commands);
        assert_region("known_tab", &mut screen, &view, |areas| areas.tab_box_rect);
    }

    #[test]
    fn the_open_tabs_key_hides_the_tab_box() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        press(&mut screen, KeyCode::F(3), &commands);
        press(&mut screen, KeyCode::F(3), &commands);
        assert_screen("tab_box_hidden", &mut screen, Some(&view));
    }

    #[test]
    fn f1_asks_for_help_and_esc_closes_its_popup() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
        let before_help = screen_text(&mut screen, Some(&view), 80, 24);
        assert_eq!(
            press(&mut screen, KeyCode::F(1), &commands),
            Some(KeyRequest::Send("help".into()))
        );
        screen.show(&serde_json::from_str(HELP_REPLY).unwrap());
        assert_screen("help_popup", &mut screen, Some(&view));
        press(&mut screen, KeyCode::Esc, &commands);
        assert_eq!(screen_text(&mut screen, Some(&view), 80, 24), before_help);
    }

    #[test]
    fn the_windows_own_map_reply_fills_nearby_and_stays_out_of_the_transcript() {
        let mut screen = Screen::new(true);
        let (view, commands) = seat_rowan_carrying_oil(&mut screen);
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
        assert_region("nearby", &mut screen, &view, |areas| {
            Some(areas.place_panel_rect)
        });
        type_text(&mut screen, "map", &commands);
        assert_eq!(
            press(&mut screen, KeyCode::Enter, &commands),
            Some(KeyRequest::Send("map".into()))
        );
        assert_eq!(screen.take_own_line(), None);
        screen.record_request_id("c11".into());
        screen.show(&map_reply("c11"));
        let transcript_after = region_text(&mut screen, Some(&view), transcript_rect);
        assert!(transcript_after.contains("Gate Yard — 0 ticks"));
    }

    #[test]
    fn a_wide_terminal_shows_the_story_the_place_and_the_tab_box_side_by_side() {
        let mut screen = Screen::new(true);
        let (view, _) = seat_rowan_carrying_oil(&mut screen);
        send_own_map(&mut screen, "c8");
        screen.show(&map_reply("c8"));
        assert_screen_at("wide", &mut screen, Some(&view), 140, 40);
    }

    #[test]
    fn the_picker_lists_each_pc_by_name() {
        let mut screen = Screen::new(true);
        let hello = (RECORDING.lines())
            .map(|row| serde_json::from_str::<Value>(row).unwrap())
            .find(|row| row["seat"] == "player:rowan" && row["get"]["body"]["kind"] == "hello")
            .unwrap();
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
