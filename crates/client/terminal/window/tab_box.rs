//! The tab box: one tab at a time of the PC's meters, what it carries, or the notes it reads,
//! and hidden when the player presses the open tab's key.

use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::symbols;
use ratatui::widgets::{Block, BorderType, LineGauge, List, ListItem, Paragraph, Tabs, Wrap};
use ratatui::Frame;

use super::{Palette, AMBER, ASH, GOLD, MULBERRY, PARCHMENT, SCARLET, TANGERINE};
use crate::protocol;
use crate::text;

/// The meter whose gauge fills in scarlet. Every other meter's fills in gold.
const HP_METER_ID: &str = "hp";

/// What the tab box shows: the PC's meters, what it carries, or what it knows.
#[derive(Clone, Copy, PartialEq)]
pub(super) enum Tab {
    Stats,
    Carried,
    Known,
}

impl Tab {
    /// Every tab in the order the tab row shows it, which is its key's order: F2, F3, F4.
    pub(super) const ALL: [Tab; 3] = [Tab::Stats, Tab::Carried, Tab::Known];

    fn title(self) -> &'static str {
        match self {
            Tab::Stats => "Stats",
            Tab::Carried => "Carried",
            Tab::Known => "Known",
        }
    }
}

pub(super) struct TabBox {
    /// The tab the box shows, `None` while the player has it hidden.
    open_tab: Option<Tab>,
}

impl TabBox {
    pub(super) fn new() -> Self {
        Self {
            open_tab: Some(Tab::Stats),
        }
    }

    /// Shows `tab`, or hides the box when `tab` is the one it shows.
    pub(super) fn toggle(&mut self, tab: Tab) {
        self.open_tab = (self.open_tab != Some(tab)).then_some(tab);
    }

    /// How many rows the Stats tab needs: one per meter, then `The world` and one per world
    /// meter. Every tab gets this many in the compact layout, so switching tabs keeps the box's
    /// size. `None` while the box is hidden.
    pub(super) fn content_rows(&self, view: Option<&protocol::View>) -> Option<u16> {
        self.open_tab?;
        let rows = view.map_or(0, |view| {
            let world_rows = match view.world_meters.len() {
                0 => 0,
                world_meter_count => world_meter_count + 1,
            };
            view.meters.len() + world_rows
        });
        Some(u16::try_from(rows.max(1)).unwrap_or(u16::MAX))
    }

    /// The tab row on the top border, the open tab's content under it, and the tab keys on the
    /// bottom border.
    pub(super) fn draw(
        &self,
        frame: &mut Frame,
        area: Rect,
        view: Option<&protocol::View>,
        palette: Palette,
    ) {
        let Some(open_tab) = self.open_tab else {
            return;
        };
        let border = Block::bordered()
            .border_type(BorderType::Rounded)
            .border_style(palette.style(MULBERRY))
            .title_bottom(palette.span("F2 F3 F4 switch · same key hides", TANGERINE));
        let content_area = border.inner(area);
        let tabs_area = Rect {
            y: area.y,
            height: 1,
            ..content_area
        };
        frame.render_widget(border, area);
        let tab_titles = Tab::ALL.map(|tab| palette.span(tab.title(), TANGERINE));
        let tabs = Tabs::new(tab_titles)
            .select(open_tab as usize)
            .highlight_style(palette.style(AMBER).add_modifier(Modifier::BOLD))
            .divider(palette.span("│", MULBERRY));
        frame.render_widget(tabs, tabs_area);
        let Some(view) = view else {
            return;
        };
        match open_tab {
            Tab::Stats => draw_meters(frame, content_area, view, palette),
            Tab::Carried => draw_carried_items(frame, content_area, view, palette),
            Tab::Known => draw_notes(frame, content_area, view, palette),
        }
    }
}

/// Each meter's gauge, then `The world` and the world's meters.
fn draw_meters(frame: &mut Frame, area: Rect, view: &protocol::View, palette: Palette) {
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
                frame.render_widget(Paragraph::new(palette.span(heading, ASH)), row);
            }
        }
        for (meter, row) in meters.iter().zip(&mut rows) {
            frame.render_widget(render_meter_gauge(meter, label_width, palette), row);
        }
    }
}

/// `Health: 8/16` padded to `label_width`, then the meter's fill: scarlet for Hp, gold for any
/// other, on mulberry.
fn render_meter_gauge(
    meter: &protocol::MeterView,
    label_width: usize,
    palette: Palette,
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
        .label(palette.span(label, PARCHMENT))
        .filled_symbol(symbols::line::THICK_HORIZONTAL)
        .filled_style(palette.style(fill_color))
        .unfilled_style(palette.style(MULBERRY))
}

/// Each item the PC carries, the wielded one in gold.
fn draw_carried_items(frame: &mut Frame, area: Rect, view: &protocol::View, palette: Palette) {
    let mut item_list_items: Vec<ListItem> = (view.items_carried.iter())
        .map(|item| {
            let is_wielded = view.wielded_item_id.as_ref() == Some(&item.id);
            let color = if is_wielded { GOLD } else { PARCHMENT };
            let line = text::render_carried_item(item, is_wielded);
            ListItem::new(palette.span(line, color))
        })
        .collect();
    if item_list_items.is_empty() {
        item_list_items.push(ListItem::new(palette.span("Nothing", ASH)));
    }
    frame.render_widget(List::new(item_list_items), area);
}

/// The PC's notes, worded as the line client words them.
fn draw_notes(frame: &mut Frame, area: Rect, view: &protocol::View, palette: Palette) {
    let notes = match text::render_notes(&view.notes) {
        Some(section) => Paragraph::new(section).style(palette.style(PARCHMENT)),
        None => Paragraph::new(palette.span("Nothing yet", ASH)),
    };
    frame.render_widget(notes.wrap(Wrap { trim: true }), area);
}
