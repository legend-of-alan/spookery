//! The place panel, drawn from the view alone.

use ratatui::layout::Rect;
use ratatui::text::Line;
use ratatui::widgets::{Paragraph, Wrap};
use ratatui::Frame;

use super::{Palette, AMBER, ASH, GOLD, ORANGE, PARCHMENT, ROSE, TANGERINE};
use crate::protocol;
use crate::text;

/// Who is here, the exits, the places past them that the map knows, and the items here. A
/// section is left out, with all after it, when its heading and first row don't both fit.
/// While the tab box is hidden, the bottom border names the keys that bring it back.
pub(super) fn draw(
    frame: &mut Frame,
    area: Rect,
    view: Option<&protocol::View>,
    is_tab_box_hidden: bool,
    palette: Palette,
) {
    let title = view.map_or("Here", |view| view.place.name.as_str());
    let mut border = palette.titled_border(title, false);
    if is_tab_box_hidden {
        border = border.title_bottom(palette.span("F2 F3 F4 tabs", TANGERINE));
    }
    let Some(view) = view else {
        frame.render_widget(border, area);
        return;
    };
    let mut here_lines: Vec<Line> = Vec::new();
    if !view.busy.is_empty() {
        here_lines.push(Line::styled(view.busy.clone(), palette.style(ASH)));
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
    for exit in &view.place.exits {
        let color = if exit.locked { ROSE } else { ORANGE };
        exit_lines.push(Line::styled(text::render_exit(exit), palette.style(color)));
    }
    if view.place.exits.is_empty() {
        exit_lines.push(Line::styled("none", palette.style(ASH)));
    }
    let mut sections = vec![here_lines, exit_lines];
    if !view.nearby.is_empty() {
        let heading = Line::from(palette.span("Nearby", ASH));
        let nearby_lines = (view.nearby.iter())
            .map(|place| render_nearby_place(place, &view.place.place_id, palette));
        sections.push(std::iter::once(heading).chain(nearby_lines).collect());
    }
    if !view.items.is_empty() {
        let heading = Line::from(palette.span("Items here", ASH));
        let item_lines = (view.items.iter())
            .map(|item| Line::styled(text::render_item(item), palette.style(PARCHMENT)));
        sections.push(std::iter::once(heading).chain(item_lines).collect());
    }
    let inner_width = border.inner(area).width;
    let count_rows = |line: &Line<'static>| {
        Paragraph::new(line.clone())
            .wrap(Wrap { trim: true })
            .line_count(inner_width)
    };
    let mut rows_left = usize::from(border.inner(area).height);
    let mut panel_lines: Vec<Line> = Vec::new();
    for section in sections {
        if section.iter().take(2).map(count_rows).sum::<usize>() > rows_left {
            break;
        }
        let section_rows: usize = section.iter().map(count_rows).sum();
        rows_left = rows_left.saturating_sub(section_rows);
        panel_lines.extend(section);
    }
    let panel = Paragraph::new(panel_lines)
        .wrap(Wrap { trim: true })
        .block(border);
    frame.render_widget(panel, area);
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
