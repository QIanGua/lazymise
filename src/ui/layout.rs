use ratatui::layout::{Constraint, Direction, Layout, Rect};

use crate::action::{FocusArea, LogPanelMode, Panel, ScreenMode};

/// Threshold below which the workbench degrades to a single-pane narrow layout.
pub const NARROW_WIDTH_THRESHOLD: u16 = 96;

/// Terminal height below which collapsed accordion cards fold into 1-line title strips.
pub const COMPACT_ACCORDION_HEIGHT: u16 = 26;

/// Semantic hit target returned by `LayoutMap::hit_test(col, row)`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HitTarget {
    HeaderProfile,
    HeaderPanel(Panel),
    SidebarPanel {
        panel: Panel,
        row_index: Option<usize>,
    },
    MainSubTab(usize),
    MainBody,
    LogDrawer {
        row_index: Option<usize>,
    },
    Overlay,
}

/// Interactive region map populated during `ui::render` and queried by mouse event handling.
#[derive(Debug, Default, Clone)]
pub struct LayoutMap {
    pub header: Rect,
    pub header_profile: Rect,
    pub header_panels: Vec<(Panel, Rect)>,
    pub sidebar_panels: Vec<(Panel, Rect, Rect)>, // (panel, outer_rect, inner_list_rect)
    pub main_area: Rect,
    pub main_inner: Rect,
    pub main_subtabs: Vec<(usize, Rect)>,
    pub log_area: Rect,
    pub log_inner: Rect,
    pub footer: Rect,
    pub overlay_area: Option<Rect>,
    pub is_narrow: bool,
}

impl LayoutMap {
    pub fn clear(&mut self) {
        self.header = Rect::default();
        self.header_profile = Rect::default();
        self.header_panels.clear();
        self.sidebar_panels.clear();
        self.main_area = Rect::default();
        self.main_inner = Rect::default();
        self.main_subtabs.clear();
        self.log_area = Rect::default();
        self.log_inner = Rect::default();
        self.footer = Rect::default();
        self.overlay_area = None;
        self.is_narrow = false;
    }

    pub fn register_sidebar_panel(&mut self, panel: Panel, outer: Rect, inner_list: Rect) {
        self.sidebar_panels.push((panel, outer, inner_list));
    }

    pub fn contains(rect: Rect, col: u16, row: u16) -> bool {
        rect.width > 0
            && rect.height > 0
            && col >= rect.x
            && col < rect.x.saturating_add(rect.width)
            && row >= rect.y
            && row < rect.y.saturating_add(rect.height)
    }

    pub fn hit_test(&self, col: u16, row: u16, list_scroll_offset: usize) -> Option<HitTarget> {
        if let Some(ov) = self.overlay_area
            && Self::contains(ov, col, row)
        {
            return Some(HitTarget::Overlay);
        }
        if Self::contains(self.header_profile, col, row) {
            return Some(HitTarget::HeaderProfile);
        }
        for (panel, rect) in &self.header_panels {
            if Self::contains(*rect, col, row) {
                return Some(HitTarget::HeaderPanel(*panel));
            }
        }
        for (idx, rect) in &self.main_subtabs {
            if Self::contains(*rect, col, row) {
                return Some(HitTarget::MainSubTab(*idx));
            }
        }
        for (panel, outer, inner) in &self.sidebar_panels {
            if Self::contains(*outer, col, row) {
                let row_index = if Self::contains(*inner, col, row) {
                    Some(list_scroll_offset + (row - inner.y) as usize)
                } else {
                    None
                };
                return Some(HitTarget::SidebarPanel {
                    panel: *panel,
                    row_index,
                });
            }
        }
        if Self::contains(self.log_area, col, row) {
            let row_index = if Self::contains(self.log_inner, col, row) {
                Some((row - self.log_inner.y) as usize)
            } else {
                None
            };
            return Some(HitTarget::LogDrawer { row_index });
        }
        if Self::contains(self.main_area, col, row) {
            return Some(HitTarget::MainBody);
        }
        None
    }

    /// Which focus area is currently hovered (used for hover wheel scrolling).
    pub fn hovered_focus(&self, col: u16, row: u16) -> Option<FocusArea> {
        if Self::contains(self.log_area, col, row) {
            return Some(FocusArea::Log);
        }
        if Self::contains(self.main_area, col, row) {
            return Some(FocusArea::Main);
        }
        if self
            .sidebar_panels
            .iter()
            .any(|(_, outer, _)| Self::contains(*outer, col, row))
        {
            return Some(FocusArea::Sidebar);
        }
        None
    }
}

/// Computed frame layout rects for Normal / Half / Full / Narrow responsive modes.
#[derive(Debug, Default, Clone)]
pub struct ComputedLayout {
    pub header: Rect,
    pub sidebar: Option<Rect>,
    pub main: Option<Rect>,
    pub log: Option<Rect>,
    pub footer: Rect,
    pub is_narrow: bool,
}

/// Accordion slot allocated to each of the 4 sidebar panels on wide screens.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AccordionSlot {
    pub panel: Panel,
    pub rect: Rect,
    pub expanded: bool,
    pub one_line: bool,
}

/// Compute top-level frame layout across Normal / Half / Full zoom modes and wide/narrow widths.
pub fn compute_layout(
    area: Rect,
    focus: FocusArea,
    screen_mode: ScreenMode,
    log_mode: LogPanelMode,
) -> ComputedLayout {
    let is_narrow = area.width < NARROW_WIDTH_THRESHOLD;

    // Reserve at least 5 rows for the main body whenever possible.
    let max_drawer = area.height.saturating_sub(7);
    let log_height = match (log_mode, screen_mode) {
        (_, ScreenMode::Full) => 0,
        (LogPanelMode::Hidden, _) => 0,
        (LogPanelMode::Compact, ScreenMode::Half) if focus == FocusArea::Log => {
            10.min(area.height / 2).min(max_drawer)
        }
        (LogPanelMode::Compact, _) => {
            if max_drawer < 3 {
                0
            } else {
                4.min(max_drawer)
            }
        }
        (LogPanelMode::Expanded, _) => {
            if max_drawer < 4 {
                max_drawer.min(3)
            } else {
                10.min(area.height / 2).min(max_drawer).max(4)
            }
        }
    };

    let v_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(log_height),
            Constraint::Length(1),
        ])
        .split(area);

    let body = v_chunks[1];
    let drawer_log = (log_height > 0).then_some(v_chunks[2]);

    let (sidebar, main, log) = if screen_mode == ScreenMode::Full {
        match focus {
            FocusArea::Sidebar => (Some(body), None, None),
            FocusArea::Main => (None, Some(body), None),
            FocusArea::Log => (None, None, Some(body)),
        }
    } else if is_narrow {
        match focus {
            FocusArea::Sidebar => (Some(body), None, drawer_log),
            FocusArea::Main => (None, Some(body), drawer_log),
            FocusArea::Log => (None, None, Some(body)),
        }
    } else {
        let left_pct = match (screen_mode, focus) {
            (ScreenMode::Half, FocusArea::Sidebar) => 58,
            (ScreenMode::Half, FocusArea::Main) => 28,
            _ => 36,
        };
        let h_chunks = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(left_pct),
                Constraint::Percentage(100 - left_pct),
            ])
            .split(body);
        (Some(h_chunks[0]), Some(h_chunks[1]), drawer_log)
    };

    ComputedLayout {
        header: v_chunks[0],
        sidebar,
        main,
        log,
        footer: v_chunks[3],
        is_narrow,
    }
}

/// Split the left sidebar vertically into 4 accordion slots (`Tasks`, `Tools`, `Env`, `Config`).
///
/// - The active sidebar panel gets `Constraint::Fill(1)`.
/// - The 3 non-active panels collapse into 3-row live summary cards when height allows
///   (`terminal_height >= 26` and `sidebar.height >= 16`), or compact 1-line title strips
///   on shorter terminals so nothing is ever crushed.
/// - When `active_panel == Panel::Jobs`, `Panel::Tasks` remains the expanded reference panel.
pub fn compute_accordion_slots(
    sidebar: Rect,
    active_panel: Panel,
    terminal_height: u16,
) -> [AccordionSlot; 4] {
    let expanded_panel = if Panel::SIDEBAR_PANELS.contains(&active_panel) {
        active_panel
    } else {
        Panel::Tasks
    };

    let one_line = terminal_height < COMPACT_ACCORDION_HEIGHT || sidebar.height < 16;
    let collapsed_height: u16 = if one_line { 1 } else { 3 };

    // On ultra-tight heights (< 6 rows), give all rows to the expanded panel and 0 to others.
    let tight_overflow = sidebar.height < 6;
    let constraints: [Constraint; 4] = Panel::SIDEBAR_PANELS.map(|panel| {
        if panel == expanded_panel {
            Constraint::Fill(1)
        } else if tight_overflow {
            Constraint::Length(0)
        } else {
            Constraint::Length(collapsed_height)
        }
    });

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(sidebar);

    [
        AccordionSlot {
            panel: Panel::Tasks,
            rect: chunks[0],
            expanded: expanded_panel == Panel::Tasks,
            one_line,
        },
        AccordionSlot {
            panel: Panel::Tools,
            rect: chunks[1],
            expanded: expanded_panel == Panel::Tools,
            one_line,
        },
        AccordionSlot {
            panel: Panel::Env,
            rect: chunks[2],
            expanded: expanded_panel == Panel::Env,
            one_line,
        },
        AccordionSlot {
            panel: Panel::Config,
            rect: chunks[3],
            expanded: expanded_panel == Panel::Config,
            one_line,
        },
    ]
}

#[cfg(test)]
mod tests {
    use pretty_assertions::assert_eq;

    use super::*;

    #[test]
    fn computes_wide_narrow_half_and_full_layouts() {
        let wide = Rect::new(0, 0, 140, 40);
        let normal = compute_layout(wide, FocusArea::Sidebar, ScreenMode::Normal, LogPanelMode::Compact);
        assert!(!normal.is_narrow);
        assert_eq!(normal.header.height, 1);
        assert_eq!(normal.footer.height, 1);
        assert_eq!(normal.log.unwrap().height, 4);
        assert!(normal.sidebar.is_some());
        assert!(normal.main.is_some());
        assert!(normal.sidebar.unwrap().width < normal.main.unwrap().width);

        let half_sidebar = compute_layout(wide, FocusArea::Sidebar, ScreenMode::Half, LogPanelMode::Compact);
        assert!(half_sidebar.sidebar.unwrap().width > half_sidebar.main.unwrap().width);

        let full_main = compute_layout(wide, FocusArea::Main, ScreenMode::Full, LogPanelMode::Compact);
        assert!(full_main.sidebar.is_none());
        assert!(full_main.log.is_none());
        assert_eq!(full_main.main.unwrap().width, 140);

        let narrow = Rect::new(0, 0, 80, 24);
        let narrow_side = compute_layout(narrow, FocusArea::Sidebar, ScreenMode::Normal, LogPanelMode::Hidden);
        assert!(narrow_side.is_narrow);
        assert!(narrow_side.sidebar.is_some());
        assert!(narrow_side.main.is_none());
        assert!(narrow_side.log.is_none());
    }

    #[test]
    fn accordion_compresses_to_one_line_on_short_terminals() {
        let tall = compute_accordion_slots(Rect::new(0, 1, 50, 34), Panel::Tools, 40);
        assert!(!tall[0].expanded && tall[0].rect.height == 3 && !tall[0].one_line);
        assert!(tall[1].expanded && tall[1].rect.height == 25);

        let short = compute_accordion_slots(Rect::new(0, 1, 50, 18), Panel::Env, 24);
        assert!(short[0].one_line && short[0].rect.height == 1);
        assert!(short[2].expanded && short[2].rect.height == 15);
    }

    #[test]
    fn layout_map_hit_tests_sidebar_subtabs_and_overlay_priority() {
        let mut map = LayoutMap {
            header_profile: Rect::new(20, 0, 14, 1),
            main_area: Rect::new(40, 1, 60, 20),
            ..Default::default()
        };
        map.main_subtabs.push((1, Rect::new(60, 1, 8, 1)));
        map.register_sidebar_panel(Panel::Tasks, Rect::new(0, 1, 40, 20), Rect::new(1, 2, 38, 18));

        assert_eq!(map.hit_test(22, 0, 0), Some(HitTarget::HeaderProfile));
        assert_eq!(map.hit_test(62, 1, 0), Some(HitTarget::MainSubTab(1)));
        assert_eq!(
            map.hit_test(5, 4, 3),
            Some(HitTarget::SidebarPanel {
                panel: Panel::Tasks,
                row_index: Some(5),
            })
        );
        assert_eq!(map.hovered_focus(50, 10), Some(FocusArea::Main));

        map.overlay_area = Some(Rect::new(10, 2, 80, 15));
        assert_eq!(map.hit_test(50, 10, 0), Some(HitTarget::Overlay));
    }
}
