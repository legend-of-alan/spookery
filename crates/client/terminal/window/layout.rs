//! Where each part of the screen goes at one terminal size.

use ratatui::layout::{Constraint, Layout, Rect};

/// The terminal width from which the place panel and the tab box stand side by side.
const WIDE_LAYOUT_COLUMNS: u16 = 120;

/// How many columns the place panel and the tab box each take.
const SIDE_PANEL_COLUMNS: u16 = 34;

/// Where each part of the screen goes at one terminal size. `ScreenAreas::new` is the one place
/// that picks it.
pub(super) struct ScreenAreas {
    pub(super) title_rect: Rect,
    pub(super) rule_rect: Rect,
    /// The rows between the rule and the input, where the PC picker goes.
    pub(super) middle_rect: Rect,
    pub(super) transcript_rect: Rect,
    pub(super) place_panel_rect: Rect,
    /// `None` while the player has the tab box hidden.
    pub(super) tab_box_rect: Option<Rect>,
    pub(super) input_rect: Rect,
    pub(super) hotkey_bar_rect: Rect,
    /// From `WIDE_LAYOUT_COLUMNS`, where the transcript shows round dividers.
    pub(super) is_wide: bool,
}

impl ScreenAreas {
    /// The compact layout below `WIDE_LAYOUT_COLUMNS`: the transcript, and a right column with
    /// the place panel above the tab box, which gets `tab_box_content_rows` and its borders.
    /// The wide layout: the transcript, the place panel and the tab box side by side.
    /// `tab_box_content_rows` is `None` while the tab box is hidden, which gives its rows to the
    /// place panel, or in the wide layout its columns to the transcript.
    pub(super) fn new(area: Rect, tab_box_content_rows: Option<u16>) -> Self {
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
