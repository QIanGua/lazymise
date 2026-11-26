use std::time::Instant;

use crossterm::event::{MouseButton, MouseEvent, MouseEventKind};

use crate::action::{Action, ConfigSubView, EnvSubView, FocusArea, Panel, TaskSubView, ToolSubView};
use crate::app::reducer::apply_action;
use crate::app::state::{AppState, DOUBLE_CLICK_WINDOW, Overlay};
use crate::mise::loader::MiseLoader;
use crate::runner::BackgroundRunner;
use crate::ui::layout::{HitTarget, LayoutMap};

/// Handle mouse click, double-click, and hover wheel scroll using `LayoutMap`.
pub fn handle_mouse(
    state: &mut AppState,
    ev: MouseEvent,
    layout: &LayoutMap,
    loader: &MiseLoader,
    runner: &BackgroundRunner,
) {
    // When an overlay is open, clicking outside closes it; wheel scrolls selection where applicable
    if !matches!(state.overlay, Overlay::None) {
        if let MouseEventKind::Down(MouseButton::Left) = ev.kind
            && let Some(ov_rect) = layout.overlay_area
            && !LayoutMap::contains(ov_rect, ev.column, ev.row)
        {
            state.overlay = Overlay::None;
            state.dirty = true;
        }
        return;
    }

    match ev.kind {
        MouseEventKind::ScrollDown => {
            handle_wheel(state, ev.column, ev.row, 3, layout, loader, runner);
        }
        MouseEventKind::ScrollUp => {
            handle_wheel(state, ev.column, ev.row, -3, layout, loader, runner);
        }
        MouseEventKind::Down(MouseButton::Left) => {
            handle_left_click(state, ev.column, ev.row, layout, loader, runner);
        }
        _ => {}
    }
}

fn handle_wheel(
    state: &mut AppState,
    col: u16,
    row: u16,
    delta: isize,
    layout: &LayoutMap,
    loader: &MiseLoader,
    runner: &BackgroundRunner,
) {
    let target = layout.hovered_focus(col, row).unwrap_or(state.focus);
    match target {
        FocusArea::Main => {
            state.detail_scroll.scroll_by(delta);
            state.dirty = true;
        }
        FocusArea::Log => {
            let prev_focus = state.focus;
            state.focus = FocusArea::Log;
            let step = if delta > 0 { 1 } else { -1 };
            move_panel_cursor(state, Panel::Jobs, step);
            state.focus = prev_focus;
            state.dirty = true;
        }
        FocusArea::Sidebar => {
            // Find which sidebar panel is hovered
            let hovered_panel = layout
                .sidebar_panels
                .iter()
                .find(|(_, outer, _)| LayoutMap::contains(*outer, col, row))
                .map(|(p, _, _)| *p)
                .unwrap_or(state.active_panel);
            if hovered_panel != state.active_panel {
                apply_action(state, Action::FocusPanel(hovered_panel), loader, runner);
            }
            let step = if delta > 0 { 1 } else { -1 };
            move_panel_cursor(state, hovered_panel, step);
            state.preload_selected_content();
            state.dirty = true;
        }
    }
}

fn handle_left_click(
    state: &mut AppState,
    col: u16,
    row: u16,
    layout: &LayoutMap,
    loader: &MiseLoader,
    runner: &BackgroundRunner,
) {
    let now = Instant::now();
    let is_double_click = state.last_click.is_some_and(|(at, lc, lr)| {
        lc == col && lr == row && now.saturating_duration_since(at) <= DOUBLE_CLICK_WINDOW
    });
    state.last_click = Some((now, col, row));

    let sidebar_panel = if state.active_panel == Panel::Jobs {
        Panel::Tasks
    } else {
        state.active_panel
    };
    let list_offset = state.list_states.get(sidebar_panel).offset();

    let Some(hit) = layout.hit_test(col, row, list_offset) else {
        return;
    };

    match hit {
        HitTarget::Overlay => {}
        HitTarget::HeaderProfile => {
            apply_action(state, Action::OpenProfileSwitcher, loader, runner);
        }
        HitTarget::HeaderPanel(panel) => {
            apply_action(state, Action::FocusPanel(panel), loader, runner);
        }
        HitTarget::MainSubTab(idx) => {
            state.focus = FocusArea::Main;
            state.detail_scroll.scroll_top();
            match state.active_panel {
                Panel::Tasks => {
                    if let Some(&sv) = TaskSubView::ALL.get(idx) {
                        state.task_subview = sv;
                    }
                }
                Panel::Tools => {
                    if let Some(&sv) = ToolSubView::ALL.get(idx) {
                        state.tool_subview = sv;
                    }
                }
                Panel::Env => {
                    if let Some(&sv) = EnvSubView::ALL.get(idx) {
                        state.env_subview = sv;
                    }
                }
                Panel::Config => {
                    if let Some(&sv) = ConfigSubView::ALL.get(idx) {
                        state.config_subview = sv;
                    }
                }
                Panel::Jobs => {}
            }
            // Trigger lazy data fetch for newly clicked sub-view
            crate::app::reducer::ensure_active_data_loaded(state, loader);
            state.dirty = true;
        }
        HitTarget::MainBody => {
            state.focus = FocusArea::Main;
            state.dirty = true;
        }
        HitTarget::SidebarPanel { panel, row_index } => {
            if state.active_panel != panel || state.focus != FocusArea::Sidebar {
                apply_action(state, Action::FocusPanel(panel), loader, runner);
            }
            if let Some(idx) = row_index {
                let len = state.current_list_len(panel);
                if len > 0 {
                    let clamped = idx.min(len - 1);
                    state.list_states.get_mut(panel).select(Some(clamped));
                    state.detail_scroll.scroll_top();
                    state.preload_selected_content();
                    crate::app::reducer::ensure_active_data_loaded(state, loader);
                    if is_double_click {
                        let dbl_action = match panel {
                            Panel::Tasks => Action::TaskRunBackground,
                            Panel::Tools => Action::ToolUpgradeSelected,
                            Panel::Env => Action::EnvToggleReveal,
                            Panel::Config => Action::ConfigEditSelected,
                            Panel::Jobs => Action::JobInspect,
                        };
                        apply_action(state, dbl_action, loader, runner);
                    }
                }
            }
            state.dirty = true;
        }
        HitTarget::LogDrawer { row_index } => {
            state.focus = FocusArea::Log;
            if let Some(idx) = row_index {
                let offset = state.list_states.jobs.offset();
                let len = state.current_list_len(Panel::Jobs);
                if len > 0 {
                    let target = (offset + idx).min(len - 1);
                    state.list_states.jobs.select(Some(target));
                    if let Some(entry) = state.selected_command_log_entry() {
                        state.selected_job_id = entry.job_id;
                    }
                    if is_double_click {
                        apply_action(state, Action::JobInspect, loader, runner);
                    }
                }
            }
            state.dirty = true;
        }
    }
}

fn move_panel_cursor(state: &mut AppState, panel: Panel, delta: isize) {
    let len = state.current_list_len(panel);
    if len == 0 {
        state.list_states.get_mut(panel).select(None);
        return;
    }
    let cur = state.list_states.get(panel).selected().unwrap_or(0) as isize;
    let next = (cur + delta).clamp(0, (len - 1) as isize) as usize;
    state.list_states.get_mut(panel).select(Some(next));
    if panel == Panel::Jobs {
        state.selected_job_id = state.selected_command_log_entry().and_then(|e| e.job_id);
    } else {
        state.detail_scroll.scroll_top();
    }
}
