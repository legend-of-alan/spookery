//! The place panel: who is here, the exits, the places past them that the map knows, and the
//! items here. It keeps the places of the last `map` reply, and asks for the map itself once the
//! PC arrives somewhere, keeping that reply out of the transcript.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::{Palette, AMBER, ASH, GOLD, ORANGE, PARCHMENT, ROSE, TANGERINE};
use crate::protocol::{self, ServerBody};
use crate::text;

#[derive(Default)]
pub(super) struct PlacePanel {
    /// The places of the last `map` reply.
    place_views: Vec<protocol::PlaceView>,
    /// Whether the PC has arrived somewhere since the window last sent its own `map`.
    is_map_due: bool,
    /// Whether the last line the window handed play is its own `map`, until play says which
    /// request id it went under.
    is_own_map_unsent: bool,
    /// The request ids of the window's own `map` requests that have no reply yet.
    own_map_request_ids: Vec<String>,
}

impl PlacePanel {
    /// Asks for the map at the next line, since the PC has arrived somewhere.
    pub(super) fn mark_map_due(&mut self) {
        self.is_map_due = true;
    }

    /// `map` when the PC has arrived somewhere since the last one, so the panel can list what
    /// lies past the exits.
    pub(super) fn take_map_line(&mut self) -> Option<String> {
        self.is_own_map_unsent = std::mem::take(&mut self.is_map_due);
        self.is_own_map_unsent.then(|| "map".into())
    }

    /// Takes the request id play sent the last line under, which marks the reply to the
    /// window's own `map`.
    pub(super) fn record_request_id(&mut self, request_id: String) {
        if std::mem::take(&mut self.is_own_map_unsent) {
            self.own_map_request_ids.push(request_id);
        }
    }

    /// Keeps the places of a `map` reply.
    pub(super) fn keep_map_places(&mut self, body: &ServerBody) {
        if let ServerBody::Map { places } = body {
            self.place_views = places.clone();
        }
    }

    /// Whether `reply_to` names one of the window's own `map` requests, whose reply stays out
    /// of the transcript. The panel then forgets that request.
    pub(super) fn remove_own_map_request(&mut self, reply_to: Option<&str>) -> bool {
        let own_reply_index = (self.own_map_request_ids.iter())
            .position(|request_id| reply_to == Some(request_id.as_str()));
        let Some(index) = own_reply_index else {
            return false;
        };
        self.own_map_request_ids.remove(index);
        true
    }

    /// Who is here, the exits, the places past them that the map knows, and the items here. A
    /// section is left out, with all after it, when its heading and first row don't both fit.
    /// While the tab box is hidden, the bottom border names the keys that bring it back.
    pub(super) fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: Option<&protocol::View>,
        is_tab_box_hidden: bool,
        palette: Palette,
    ) {
        let title = view.map_or("Here", |view| view.place_name.as_str());
        let mut border = palette.titled_border(title, false);
        if is_tab_box_hidden {
            border = border.title_bottom(palette.span("F2 F3 F4 tabs", TANGERINE));
        }
        let Some(view) = view else {
            frame.render_widget(border, area);
            return;
        };
        let mut here_lines: Vec<Line> = Vec::new();
        for line in [&view.cant_act, &view.busy] {
            if !line.is_empty() {
                here_lines.push(Line::styled(line.clone(), palette.style(ASH)));
            }
        }
        let mut here_spans = vec![palette.span("Here: ", PARCHMENT)];
        for (index, (id, name)) in text::actor_ids_and_names(view).enumerate() {
            if index > 0 {
                here_spans.push(palette.span(", ", PARCHMENT));
            }
            here_spans.push(if *id == view.viewer {
                palette.span("You", AMBER)
            } else {
                palette.span(name.clone(), TANGERINE)
            });
        }
        here_lines.push(Line::from(here_spans));
        let mut exit_lines = vec![Line::from(palette.span("Exits", ASH))];
        for exit in &view.exits {
            let color = if exit.locked { ROSE } else { ORANGE };
            exit_lines.push(Line::styled(text::render_exit(exit), palette.style(color)));
        }
        if view.exits.is_empty() {
            exit_lines.push(Line::styled("none", palette.style(ASH)));
        }
        let mut sections = vec![here_lines, exit_lines];
        let nearby_lines: Vec<Line> = (view.exits.iter())
            .filter_map(|exit| {
                let place =
                    (self.place_views.iter()).find(|place| place.place_id == exit.place_id)?;
                Some(render_nearby_place(place, &view.place, palette))
            })
            .collect();
        if !nearby_lines.is_empty() {
            let heading = Line::from(palette.span("Nearby", ASH));
            sections.push(std::iter::once(heading).chain(nearby_lines).collect());
        }
        if !view.items.is_empty() {
            let heading = Line::from(palette.span("Items here", ASH));
            let item_lines = (view.items.iter())
                .map(|item| Line::styled(text::render_item(item), palette.style(PARCHMENT)));
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
}

/// `Docks Street → Gate Yard`: a place an exit leads to, then where its exits go from there as
/// the map last saw them, leaving out the way back to `here_place_id`.
fn render_nearby_place(
    place: &protocol::PlaceView,
    here_place_id: &str,
    palette: Palette,
) -> Line<'static> {
    let onward_place_names: Vec<&str> = (place.exits.iter())
        .filter(|exit| exit.place_id != here_place_id)
        .map(|exit| exit.place_name.as_str())
        .collect();
    let mut spans = vec![palette.span(place.name.clone(), GOLD)];
    if !onward_place_names.is_empty() {
        let onward = format!(" → {}", onward_place_names.join(", "));
        spans.push(palette.span(onward, PARCHMENT));
    }
    Line::from(spans)
}
