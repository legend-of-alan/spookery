//! The game window `spook play` opens at a terminal: the picker, then the story, the place
//! panel, the input line and the key bar, painted in the Spookery palette. `play` hands it each
//! message body and reads from it each line the player sends.

use std::collections::BTreeMap;

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
    fallback: u8,
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

impl PaletteColor {
    const fn new(rgb: u32, fallback: u8) -> Self {
        Self { rgb, fallback }
    }

    fn to_color(self, truecolor: bool) -> Color {
        if truecolor {
            let [_, r, g, b] = self.rgb.to_be_bytes();
            Color::Rgb(r, g, b)
        } else {
            Color::Indexed(self.fallback)
        }
    }
}

/// The terminal in raw mode on the alternate screen, its key events and the screen drawn on
/// it. Dropping it restores the terminal.
pub(crate) struct Window {
    terminal: DefaultTerminal,
    keys: EventStream,
    screen: Screen,
}

impl Window {
    /// Sets up the terminal, which `ratatui` also restores when play panics. Truecolor when
    /// `COLORTERM` says so or the terminal is Windows Terminal, else the 256-color palette.
    pub(crate) fn open() -> Result<Self, Error> {
        let colorterm = std::env::var("COLORTERM").unwrap_or_default();
        let truecolor = matches!(colorterm.as_str(), "truecolor" | "24bit")
            || std::env::var_os("WT_SESSION").is_some();
        let terminal = ratatui::try_init().map_err(|err| {
            ratatui::restore();
            Error::Io(err)
        })?;
        Ok(Self {
            terminal,
            keys: EventStream::new(),
            screen: Screen::new(truecolor),
        })
    }

    /// Adds one body to the screen and draws it.
    pub(crate) fn show(&mut self, body: &ServerBody) -> Result<(), Error> {
        self.screen.show(body);
        self.draw()
    }

    /// Handles keys until the player sends a line: a PC id at the picker, else a typed line.
    /// `None` when they quit with Ctrl+C.
    pub(crate) async fn next_line(&mut self) -> Result<Option<String>, Error> {
        self.draw()?;
        while let Some(event) = self.keys.next().await {
            let request = match event.map_err(Error::Io)? {
                Event::Key(key) => self.screen.press(key),
                _ => None,
            };
            self.draw()?;
            match request {
                Some(Request::Send(line)) => return Ok(Some(line)),
                Some(Request::Quit) => return Ok(None),
                None => {}
            }
        }
        Ok(None)
    }

    fn draw(&mut self) -> Result<(), Error> {
        let screen = &mut self.screen;
        self.terminal
            .draw(|frame| screen.draw(frame))
            .map_err(Error::Io)?;
        Ok(())
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        ratatui::restore();
    }
}

/// What a key asks play to do.
#[derive(Debug, PartialEq)]
enum Request {
    /// Send this line, as the lines mode sends a typed one.
    Send(String),
    /// Release the PC and leave.
    Quit,
}

/// What the window shows: the picker until a PC is chosen, then the title bar, story, place
/// panel and input line.
struct Screen {
    truecolor: bool,
    picker: Option<Picker>,
    /// Each PC's name by id, from the hello.
    pc_names: BTreeMap<String, String>,
    /// The seated PC's name, for the title bar.
    pc_name: String,
    /// The last round's index and world time.
    last_round: Option<(u64, u64)>,
    text: text::Renderer,
    story: Vec<Line<'static>>,
    /// How many wrapped rows the story is scrolled up from its bottom.
    story_rows_back: usize,
    /// How many rows PgUp and PgDn move: the story pane's height less one, as last drawn.
    story_page_rows: usize,
    /// The place whose name and prose the story showed last.
    shown_place_id: String,
    /// The latest view, which the place panel shows.
    view: Option<protocol::View>,
    input: Input,
    /// The commands Tab completes: the hello's, but `claim`.
    commands: Vec<String>,
    /// The lines sent, oldest first, and which one Up and Down put in the input.
    history: Vec<String>,
    recalled: Option<usize>,
}

/// Every PC the hello lists, and the open one Enter would choose.
struct Picker {
    rows: Vec<text::PickerRow>,
    chosen: ListState,
}

impl Screen {
    fn new(truecolor: bool) -> Self {
        Self {
            truecolor,
            picker: None,
            pc_names: BTreeMap::new(),
            pc_name: String::new(),
            last_round: None,
            text: text::Renderer::new(),
            story: Vec::new(),
            story_rows_back: 0,
            story_page_rows: 1,
            shown_place_id: String::new(),
            view: None,
            input: Input::default(),
            commands: Vec::new(),
            history: Vec::new(),
            recalled: None,
        }
    }

    fn style(&self, color: PaletteColor) -> Style {
        Style::new().fg(color.to_color(self.truecolor))
    }

    fn span(&self, text: impl Into<String>, color: PaletteColor) -> Span<'static> {
        Span::styled(text.into(), self.style(color))
    }

    /// Adds each line of `shown` to the story in `color`.
    fn push_text(&mut self, shown: &str, color: PaletteColor) {
        let style = self.style(color);
        let lines = shown
            .lines()
            .map(|line| Line::styled(line.to_string(), style));
        self.story.extend(lines);
    }

    fn show(&mut self, body: &ServerBody) {
        match body {
            ServerBody::Hello {
                commands,
                open,
                taken,
                pc_names,
                ..
            } => {
                let rows = text::picker_rows(open, taken, pc_names);
                let first_open = rows.iter().position(|row| row.open_id.is_some());
                let chosen = ListState::default().with_selected(first_open);
                self.picker = Some(Picker { rows, chosen });
                self.pc_names = pc_names.clone();
                self.commands = commands
                    .iter()
                    .filter(|command| *command != "claim")
                    .cloned()
                    .collect();
            }
            ServerBody::Claimed { actor, .. } => {
                self.pc_name = self.pc_names.get(actor).cloned().unwrap_or_default();
                let shown = self.text.render(body);
                self.push_text(&shown, PARCHMENT);
            }
            ServerBody::Round {
                round_index,
                world_time,
            } => {
                self.last_round = Some((*round_index, *world_time));
                let divider = format!("── round {round_index} · tick {world_time} ──");
                self.story.push(Line::styled(divider, self.style(DRIP)));
            }
            ServerBody::View(view) => {
                if view.place != self.shown_place_id {
                    self.shown_place_id = view.place.clone();
                    self.push_arrival(view);
                }
                self.view = Some((**view).clone());
            }
            ServerBody::Say(speech) => {
                let line = self.speech_line(speech);
                self.story.push(line);
            }
            ServerBody::Check { outcome } => {
                let color = match outcome {
                    Outcome::Success => GOLD,
                    Outcome::Partial => EMBER,
                    Outcome::Miss => SCARLET,
                };
                let sentence = text::render(body);
                self.story
                    .push(Line::styled(format!("◆ {sentence}"), self.style(color)));
            }
            ServerBody::Refused(refusal) => {
                self.push_text(&text::render_refusal(refusal, false), ROSE);
            }
            ServerBody::Changes { .. } | ServerBody::Queued { .. } => {
                let shown = self.text.render(body);
                self.push_text(&shown, ASH);
            }
            _ => {
                let shown = self.text.render(body);
                self.push_text(&shown, PARCHMENT);
            }
        }
    }

    /// The place's name and prose, shown once when the PC arrives.
    fn push_arrival(&mut self, view: &protocol::View) {
        if !view.place_name.is_empty() {
            let name = self.span(view.place_name.clone(), GOLD);
            let marker = self.span("▌ ", FLAME);
            self.story
                .push(Line::from(vec![marker, name.add_modifier(Modifier::BOLD)]));
        }
        self.push_text(view.body.trim(), PARCHMENT);
    }

    fn speech_line(&self, speech: &Speech) -> Line<'static> {
        let speaker_color = match speech.from {
            Speaker::Player(_) => AMBER,
            Speaker::Gm => ROSE,
            Speaker::Npc(_) => TANGERINE,
        };
        Line::from(vec![
            self.span(text::speaker_for_player(speech), speaker_color),
            self.span(format!(": {}", speech.text), PARCHMENT),
        ])
    }

    /// What `key` does. Only presses count, since Windows also reports releases.
    fn press(&mut self, key: KeyEvent) -> Option<Request> {
        if key.kind != KeyEventKind::Press {
            return None;
        }
        if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
            return Some(Request::Quit);
        }
        if let Some(picker) = &mut self.picker {
            let pc_id = picker.press(key.code)?;
            self.picker = None;
            return Some(Request::Send(pc_id));
        }
        match key.code {
            KeyCode::Enter => return self.send_typed(),
            KeyCode::Tab => self.complete_command(),
            KeyCode::Up | KeyCode::Down => self.recall(key.code == KeyCode::Up),
            KeyCode::PageUp => self.story_rows_back += self.story_page_rows,
            KeyCode::PageDown => {
                self.story_rows_back = self.story_rows_back.saturating_sub(self.story_page_rows);
            }
            _ => {
                self.input.handle_event(&Event::Key(key));
            }
        }
        None
    }

    /// Sends the typed line, keeps it in the history and shows it in the story.
    fn send_typed(&mut self) -> Option<Request> {
        let line = self.input.value_and_reset();
        self.recalled = None;
        if line.trim().is_empty() {
            return None;
        }
        if self.history.last() != Some(&line) {
            self.history.push(line.clone());
        }
        let typed = Line::from(vec![self.span("› ", FLAME), self.span(line.clone(), BERRY)]);
        self.story.push(typed);
        self.story_rows_back = 0;
        Some(Request::Send(line))
    }

    /// Completes the first word to the command names that start with it: to the one name and a
    /// space, or to what all of them share.
    fn complete_command(&mut self) {
        let typed = self.input.value();
        if typed.contains(' ') {
            return;
        }
        let mut names = self
            .commands
            .iter()
            .filter(|command| command.starts_with(typed));
        let Some(first) = names.next() else {
            return;
        };
        let mut shared = first.as_str();
        let mut only = true;
        for name in names {
            only = false;
            let bytes: usize = shared
                .chars()
                .zip(name.chars())
                .take_while(|(a, b)| a == b)
                .map(|(a, _)| a.len_utf8())
                .sum();
            shared = &shared[..bytes];
        }
        let completed = if only {
            format!("{shared} ")
        } else {
            shared.to_string()
        };
        self.input = Input::new(completed);
    }

    /// Puts an older line from the history in the input, or a newer one, or clears it past the
    /// newest.
    fn recall(&mut self, older: bool) {
        if self.history.is_empty() {
            return;
        }
        let newest = self.history.len() - 1;
        self.recalled = match (self.recalled, older) {
            (None, true) => Some(newest),
            (Some(at), true) => Some(at.saturating_sub(1)),
            (Some(at), false) if at < newest => Some(at + 1),
            (_, false) => None,
        };
        let line = self.recalled.map(|at| self.history[at].clone());
        self.input = Input::new(line.unwrap_or_default());
    }

    fn draw(&mut self, frame: &mut Frame) {
        let area = frame.area();
        let void = VOID.to_color(self.truecolor);
        frame.render_widget(Block::new().style(Style::new().bg(void)), area);
        let [title, rule, middle, input, keys] = Layout::vertical([
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Fill(1),
            Constraint::Length(3),
            Constraint::Length(1),
        ])
        .areas(area);
        self.draw_title(frame, title);
        self.draw_rule(frame, rule);
        if self.picker.is_some() {
            self.draw_picker(frame, middle);
            self.draw_keys(
                frame,
                keys,
                &[("↑↓", "choose"), ("Enter", "play"), ("Ctrl+C", "quit")],
            );
            return;
        }
        let [story, place] =
            Layout::horizontal([Constraint::Fill(2), Constraint::Fill(1)]).areas(middle);
        self.draw_story(frame, story);
        self.draw_place(frame, place);
        self.draw_input(frame, input);
        self.draw_keys(
            frame,
            keys,
            &[
                ("Enter", "send"),
                ("Tab", "complete"),
                ("↑↓", "history"),
                ("PgUp PgDn", "scroll"),
                ("Ctrl+C", "release"),
            ],
        );
    }

    fn pane(&self, title: &str, focused: bool) -> Block<'static> {
        let (border, title_color) = if focused {
            (PLUM, AMBER)
        } else {
            (DRIP, TANGERINE)
        };
        Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(border))
            .title(self.span(format!(" {title} "), title_color))
    }

    /// SPOOKERY in its gradient, then the PC's name, the round and the tick.
    fn draw_title(&self, frame: &mut Frame, area: Rect) {
        let dusk = DUSK.to_color(self.truecolor);
        let mut spans: Vec<Span> = vec![Span::raw(" ")];
        spans.extend(TITLE_LETTERS.iter().map(|(letter, color)| {
            self.span(letter.to_string(), *color)
                .add_modifier(Modifier::BOLD)
        }));
        let mut seated = self.pc_name.clone();
        if let Some((round_index, world_time)) = self.last_round {
            seated.push_str(&format!(" · round {round_index} · tick {world_time}"));
        }
        let [letters, pc] =
            Layout::horizontal([Constraint::Length(10), Constraint::Fill(1)]).areas(area);
        frame.render_widget(Paragraph::new(Line::from(spans)).bg(dusk), letters);
        let pc_line = Line::styled(format!("{seated} "), self.style(PARCHMENT)).right_aligned();
        frame.render_widget(Paragraph::new(pc_line).bg(dusk), pc);
    }

    /// A plum rule of upper half blocks with drip-colored drops hanging from it.
    fn draw_rule(&self, frame: &mut Frame, area: Rect) {
        let plum = PLUM.to_color(self.truecolor);
        let drop = DRIP.to_color(self.truecolor);
        let void = VOID.to_color(self.truecolor);
        let spans: Vec<Span> = (0..area.width)
            .map(|column| {
                let below = if (column * 5 + 3) % 13 < 2 {
                    drop
                } else {
                    void
                };
                Span::styled("▀", Style::new().fg(plum).bg(below))
            })
            .collect();
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
    }

    fn draw_picker(&mut self, frame: &mut Frame, area: Rect) {
        let block = self.pane(text::PICKER_HEADING, true);
        let highlight = self.style(GOLD).add_modifier(Modifier::BOLD);
        let (parchment, ash) = (self.style(PARCHMENT), self.style(ASH));
        let Some(picker) = &mut self.picker else {
            return;
        };
        let mut items: Vec<ListItem> = picker
            .rows
            .iter()
            .map(|row| {
                let style = if row.open_id.is_some() {
                    parchment
                } else {
                    ash
                };
                ListItem::new(row.label()).style(style)
            })
            .collect();
        if picker.chosen.selected().is_none() {
            items.push(ListItem::new(text::NONE_FREE).style(ash));
        }
        let list = List::new(items)
            .block(block)
            .highlight_style(highlight)
            .highlight_symbol("› ");
        frame.render_stateful_widget(list, area, &mut picker.chosen);
    }

    /// The story, wrapped and pinned to the bottom unless scrolled back.
    fn draw_story(&mut self, frame: &mut Frame, area: Rect) {
        let block = self.pane("Story", true);
        let inner = block.inner(area);
        let story = Paragraph::new(self.story.clone()).wrap(Wrap { trim: false });
        let rows = story.line_count(inner.width);
        let page = usize::from(inner.height);
        self.story_page_rows = page.saturating_sub(1).max(1);
        let bottom = rows.saturating_sub(page);
        self.story_rows_back = self.story_rows_back.min(bottom);
        let top = u16::try_from(bottom - self.story_rows_back).unwrap_or(u16::MAX);
        frame.render_widget(story.scroll((top, 0)).block(block), area);
    }

    /// Who is here, the exits, the items, the meters, then what the PC knows and its notes.
    fn draw_place(&self, frame: &mut Frame, area: Rect) {
        let Some(view) = &self.view else {
            frame.render_widget(self.pane("Here", false), area);
            return;
        };
        let mut lines: Vec<Line> = Vec::new();
        for line in [&view.cant_act, &view.busy] {
            if !line.is_empty() {
                lines.push(Line::styled(line.clone(), self.style(ASH)));
            }
        }
        let mut here = vec![self.span("Here: ", PARCHMENT)];
        for (index, (id, name)) in view
            .visible_actors
            .iter()
            .zip(&view.actor_names)
            .enumerate()
        {
            if index > 0 {
                here.push(self.span(", ", PARCHMENT));
            }
            here.push(if *id == view.viewer {
                self.span("You", AMBER)
            } else {
                self.span(name.clone(), TANGERINE)
            });
        }
        lines.push(Line::from(here));
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
                lines.extend(meters.iter().map(|meter| self.meter_gauge(meter)));
            }
        }
        let known: Vec<&str> = view
            .known_facts
            .iter()
            .map(|fact| fact.text.as_str())
            .collect();
        let notes: Vec<&str> = view.notes.iter().map(|note| note.text.as_str()).collect();
        for (heading, sentences) in [("You know", known), ("Notes", notes)] {
            if !sentences.is_empty() {
                self.push_heading(&mut lines, heading);
                for sentence in sentences {
                    lines.push(Line::styled(sentence.to_string(), self.style(PARCHMENT)));
                }
            }
        }
        let panel = Paragraph::new(lines)
            .wrap(Wrap { trim: true })
            .block(self.pane(&view.place_name, false));
        frame.render_widget(panel, area);
    }

    fn push_heading(&self, lines: &mut Vec<Line<'static>>, heading: &str) {
        lines.push(Line::default());
        lines.push(Line::styled(heading.to_string(), self.style(ASH)));
    }

    /// `Hp: 8/16` and a gauge of its fill, scarlet on drip.
    fn meter_gauge(&self, meter: &protocol::MeterView) -> Line<'static> {
        let filled = match usize::try_from(meter.max) {
            Ok(max) if max > 0 => {
                let value = usize::try_from(meter.value.clamp(0, meter.max)).unwrap_or(0);
                (value * GAUGE_CELLS + max / 2) / max
            }
            _ => 0,
        };
        Line::from(vec![
            self.span(format!("{} ", text::meter_line(meter)), PARCHMENT),
            self.span("━".repeat(filled), SCARLET),
            self.span("─".repeat(GAUGE_CELLS - filled), DRIP),
        ])
    }

    /// The input line behind its prompt, with the cursor where the player types.
    fn draw_input(&self, frame: &mut Frame, area: Rect) {
        let block = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(self.style(DRIP));
        let [prompt, typed] = Layout::horizontal([Constraint::Length(2), Constraint::Fill(1)])
            .areas(block.inner(area));
        frame.render_widget(block, area);
        frame.render_widget(Paragraph::new(self.span("› ", FLAME)), prompt);
        let scroll = self
            .input
            .visual_scroll(usize::from(typed.width.saturating_sub(1)));
        let value = self.span(self.input.value().to_string(), PARCHMENT);
        let scroll_cells = u16::try_from(scroll).unwrap_or(u16::MAX);
        frame.render_widget(Paragraph::new(value).scroll((0, scroll_cells)), typed);
        let cursor = u16::try_from(self.input.visual_cursor() - scroll).unwrap_or(u16::MAX);
        frame.set_cursor_position((typed.x.saturating_add(cursor), typed.y));
    }

    /// Each key's name in tangerine and what it does.
    fn draw_keys(&self, frame: &mut Frame, area: Rect, keys: &[(&str, &str)]) {
        let mut spans = Vec::new();
        for (key, does) in keys {
            spans.push(self.span(format!(" {key} "), TANGERINE));
            spans.push(self.span(format!("{does} "), ASH));
        }
        let dusk = DUSK.to_color(self.truecolor);
        frame.render_widget(Paragraph::new(Line::from(spans)).bg(dusk), area);
    }
}

impl Picker {
    /// Moves the choice to the open PC above or below with Up and Down. Enter chooses it and
    /// returns its PC id.
    fn press(&mut self, code: KeyCode) -> Option<String> {
        let at = self.chosen.selected()?;
        let is_open = |index: &usize| self.rows[*index].open_id.is_some();
        let next = match code {
            KeyCode::Up => (0..at).rev().find(is_open),
            KeyCode::Down => (at + 1..self.rows.len()).find(is_open),
            KeyCode::Enter => return self.rows[at].open_id.clone(),
            _ => None,
        };
        if next.is_some() {
            self.chosen.select(next);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crossterm::event::{KeyCode, KeyEvent};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use serde_json::Value;

    use super::{Request, Screen};
    use crate::protocol::{ServerBody, ServerMessage};

    const RECORDING: &str = include_str!("../protocol/sessions/v2.jsonl");

    fn press(screen: &mut Screen, code: KeyCode) -> Option<Request> {
        screen.press(KeyEvent::from(code))
    }

    fn type_text(screen: &mut Screen, text: &str) {
        for letter in text.chars() {
            press(screen, KeyCode::Char(letter));
        }
    }

    /// Replays Rowan's side of the recorded docks session: shows each message Rowan got, picks
    /// Rowan at the picker, types each command Rowan sent, and stops before the first command
    /// named `stop`.
    fn replay_until(screen: &mut Screen, stop: &str) {
        for row in RECORDING.lines() {
            let row: Value = serde_json::from_str(row).unwrap();
            if row["seat"] != "player:rowan" {
                continue;
            }
            if let Some(sent) = row.get("send") {
                let (command, args) = (&sent["body"]["kind"], &sent["body"]["args"]);
                let typed = match command.as_str().unwrap_or_default() {
                    "claim" => continue,
                    command if command == stop => return,
                    "say" => format!("say {}", args["text"].as_str().unwrap_or_default()),
                    command => format!("{command} {}", args["target"].as_str().unwrap_or_default()),
                };
                type_text(screen, typed.trim());
                press(screen, KeyCode::Enter);
                continue;
            }
            let message: ServerMessage = serde_json::from_value(row["get"].clone()).unwrap();
            screen.show(&message.body);
            if matches!(message.body, ServerBody::Hello { .. }) {
                press(screen, KeyCode::Down);
                assert_eq!(
                    press(screen, KeyCode::Enter),
                    Some(Request::Send("rowan".into()))
                );
            }
        }
    }

    /// The screen's text at `width` by `height`, one row per line.
    fn screen_text(screen: &mut Screen, width: u16, height: u16) -> String {
        let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
        terminal.draw(|frame| screen.draw(frame)).unwrap();
        let buffer = terminal.backend().buffer();
        let rows: Vec<String> = (0..height)
            .map(|y| {
                let row: String = (0..width).map(|x| buffer[(x, y)].symbol()).collect();
                row.trim_end().to_string()
            })
            .collect();
        rows.join("\n")
    }

    fn assert_screens(name: &str, screen: &mut Screen) {
        for (width, height) in [(80, 24), (120, 40)] {
            let shown = screen_text(screen, width, height);
            insta::assert_snapshot!(format!("{name}_{width}x{height}"), shown);
        }
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
        assert_screens("picker", &mut screen);
    }

    #[test]
    fn a_seated_table_shows_the_story_and_the_place() {
        let mut screen = Screen::new(true);
        replay_until(&mut screen, "travel");
        assert_screens("seated", &mut screen);
    }

    #[test]
    fn tab_completes_a_command_name() {
        let mut screen = Screen::new(true);
        replay_until(&mut screen, "travel");
        type_text(&mut screen, "tr");
        press(&mut screen, KeyCode::Tab);
        type_text(&mut screen, "stairs");
        assert_screens("completed", &mut screen);
    }

    #[test]
    fn page_up_scrolls_the_story_back() {
        let mut screen = Screen::new(true);
        replay_until(&mut screen, "release");
        screen_text(&mut screen, 80, 24);
        press(&mut screen, KeyCode::PageUp);
        assert_screens("scrolled_back", &mut screen);
    }

    #[test]
    fn a_refusal_shows_in_the_story() {
        let mut screen = Screen::new(true);
        replay_until(&mut screen, "release");
        assert_screens("refused", &mut screen);
    }
}
