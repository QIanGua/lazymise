pub mod layout;
pub mod overlay;
pub mod panels;
pub mod theme;
pub mod widgets;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::action::{FocusArea, Panel, ScreenMode};
use crate::app::keymap::footer_hint_groups_for_state;
use crate::app::state::{AppState, Overlay};
use crate::events::JobStatus;
use crate::ui::layout::{LayoutMap, compute_accordion_slots, compute_layout};
use crate::ui::theme::glyphs;
use crate::ui::widgets::{
    format_duration_ms, render_toast_overlay, shorten_path, truncate_display,
};

/// Primary pure-rendering entrypoint for lazymise 2.0.
/// Mutates ONLY `state.list_states`, `state.detail_scroll` viewport bounds, and `layout_map`.
/// Strictly zero disk I/O and zero subprocess calls.
pub fn render(frame: &mut Frame, state: &mut AppState, layout_map: &mut LayoutMap) {
    let area = frame.area();
    layout_map.clear();

    if area.width < 24 || area.height < 6 {
        let msg = Paragraph::new("Terminal too small for lazymise (min 40x10). Please resize.")
            .style(theme::warning());
        frame.render_widget(msg, area);
        return;
    }

    let computed = compute_layout(area, state.focus, state.screen_mode, state.log_mode);
    layout_map.header = computed.header;
    layout_map.footer = computed.footer;
    layout_map.is_narrow = computed.is_narrow;

    render_header(frame, state, computed.header, computed.is_narrow, layout_map);

    if let Some(sidebar) = computed.sidebar {
        if computed.is_narrow || state.screen_mode == ScreenMode::Full {
            render_single_sidebar_panel(frame, state, state.active_panel, sidebar, layout_map);
        } else {
            let slots = compute_accordion_slots(sidebar, state.active_panel, area.height);
            for slot in slots {
                if slot.rect.height == 0 {
                    continue;
                }
                if slot.expanded {
                    render_single_sidebar_panel(frame, state, slot.panel, slot.rect, layout_map);
                } else {
                    render_collapsed_summary_card(
                        frame,
                        state,
                        slot.panel,
                        slot.rect,
                        slot.one_line,
                        layout_map,
                    );
                }
            }
        }
    }

    if let Some(main_area) = computed.main {
        layout_map.main_area = main_area;
        match state.active_panel {
            Panel::Tasks => panels::tasks::render_main(frame, state, main_area, layout_map),
            Panel::Tools => panels::tools::render_main(frame, state, main_area, layout_map),
            Panel::Env => panels::env::render_main(frame, state, main_area, layout_map),
            Panel::Config => panels::config::render_main(frame, state, main_area, layout_map),
            Panel::Jobs => panels::jobs::render_main(frame, state, main_area, layout_map),
        }
    }

    if let Some(log_area) = computed.log {
        layout_map.log_area = log_area;
        panels::jobs::render_log_drawer(
            frame,
            state,
            log_area,
            state.focus == FocusArea::Log,
            layout_map,
        );
    }

    render_footer(frame, state, computed.footer);

    // Floating top-right toast card when no modal overlay is covering the viewport.
    if matches!(state.overlay, Overlay::None) && area.height >= 12 {
        render_toast_overlay(frame, area, state);
    }

    overlay::render(frame, state, layout_map);
}

fn render_single_sidebar_panel(
    frame: &mut Frame,
    state: &mut AppState,
    panel: Panel,
    area: Rect,
    layout_map: &mut LayoutMap,
) {
    match panel {
        Panel::Tasks => panels::tasks::render_sidebar(frame, state, area, layout_map),
        Panel::Tools => panels::tools::render_sidebar(frame, state, area, layout_map),
        Panel::Env => panels::env::render_sidebar(frame, state, area, layout_map),
        Panel::Config => panels::config::render_sidebar(frame, state, area, layout_map),
        Panel::Jobs => panels::jobs::render_sidebar(frame, state, area, layout_map),
    }
}

fn render_collapsed_summary_card(
    frame: &mut Frame,
    state: &AppState,
    panel: Panel,
    area: Rect,
    one_line: bool,
    layout_map: &mut LayoutMap,
) {
    match panel {
        Panel::Tasks => panels::tasks::render_summary_card(frame, state, area, one_line, layout_map),
        Panel::Tools => panels::tools::render_summary_card(frame, state, area, one_line, layout_map),
        Panel::Env => panels::env::render_summary_card(frame, state, area, one_line, layout_map),
        Panel::Config => panels::config::render_summary_card(frame, state, area, one_line, layout_map),
        Panel::Jobs => {}
    }
}

fn render_header(
    frame: &mut Frame,
    state: &AppState,
    area: Rect,
    is_narrow: bool,
    layout_map: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let mut left_spans: Vec<Span<'static>> = Vec::new();
    let mut cursor_x = area.x;
    let max_x = area.x.saturating_add(area.width);

    if is_narrow {
        let status_span = build_header_job_status_span(state, true);
        let status_w = status_span
            .as_ref()
            .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
            .unwrap_or(0);
        let left_limit = if status_w > 0 && area.width > status_w + 16 {
            max_x.saturating_sub(status_w + 1)
        } else {
            max_x
        };
        let use_long_labels = left_limit.saturating_sub(area.x) >= 72;

        // Compact 1-5 panel switcher badges + focus indicator + ENV badge.
        for panel in Panel::ALL {
            let active = state.active_panel == panel;
            let text = if use_long_labels {
                format!("[{}:{}]", panel.number(), panel.short_title())
            } else {
                format!("[{}]", panel.number())
            };
            let w = UnicodeWidthStr::width(text.as_str()) as u16;
            if cursor_x.saturating_add(w) < left_limit {
                layout_map
                    .header_panels
                    .push((panel, Rect::new(cursor_x, area.y, w, 1)));
                left_spans.push(Span::styled(text, theme::filter_pill(active)));
                left_spans.push(Span::raw(" "));
                cursor_x = cursor_x.saturating_add(w + 1);
            }
        }

        // Focus indicator: `[LIST ←→ DETAIL]`
        let focus_text = match state.focus {
            FocusArea::Sidebar => "[LIST ←→ detail] ",
            FocusArea::Main => "[list ←→ DETAIL] ",
            FocusArea::Log => "[LOG] ",
        };
        let fw = UnicodeWidthStr::width(focus_text) as u16;
        if cursor_x.saturating_add(fw) < left_limit {
            left_spans.push(Span::styled(focus_text, theme::accent()));
            cursor_x = cursor_x.saturating_add(fw);
        }

        let profile = state.profile.as_deref().unwrap_or("default");
        let env_badge = format!("E:[{profile}]");
        let ew = UnicodeWidthStr::width(env_badge.as_str()) as u16;
        if cursor_x.saturating_add(ew) <= left_limit {
            layout_map.header_profile = Rect::new(cursor_x, area.y, ew, 1);
            left_spans.push(Span::styled(
                env_badge,
                if state.profile.is_some() {
                    theme::warning().add_modifier(Modifier::BOLD)
                } else {
                    theme::muted()
                },
            ));
            cursor_x = cursor_x.saturating_add(ew);
        }

        if let Some(span) = status_span {
            let remaining = max_x.saturating_sub(cursor_x);
            if remaining >= status_w {
                let pad = remaining.saturating_sub(status_w) as usize;
                if pad > 0 {
                    left_spans.push(Span::raw(" ".repeat(pad)));
                }
                left_spans.push(span);
            }
        }

        frame.render_widget(Paragraph::new(Line::from(left_spans)), area);
        return;
    }

    // Wide terminal header: brand + workdir + clickable ENV:[profile] + right status pills.
    let brand = " lazymise ";
    left_spans.push(Span::styled(brand, theme::brand_pill()));
    cursor_x = cursor_x.saturating_add(UnicodeWidthStr::width(brand) as u16);

    let sep = " │ ";
    left_spans.push(Span::styled(sep, theme::subtle()));
    cursor_x = cursor_x.saturating_add(UnicodeWidthStr::width(sep) as u16);

    let right_spans = build_header_right_spans(state);
    let right_w: u16 = right_spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()) as u16)
        .sum();

    let profile = state.profile.as_deref().unwrap_or("default");
    let env_badge = format!("ENV:[{profile}]");
    let env_w = UnicodeWidthStr::width(env_badge.as_str()) as u16;

    let path_budget = area
        .width
        .saturating_sub(
            (cursor_x - area.x)
                .saturating_add(env_w)
                .saturating_add(right_w)
                .saturating_add(6),
        )
        .clamp(12, 48) as usize;
    let workdir_str = shorten_path(&state.workdir.display().to_string(), path_budget);
    let workdir_w = UnicodeWidthStr::width(workdir_str.as_str()) as u16;
    left_spans.push(Span::styled(workdir_str, theme::muted()));
    cursor_x = cursor_x.saturating_add(workdir_w);

    left_spans.push(Span::styled(sep, theme::subtle()));
    cursor_x = cursor_x.saturating_add(UnicodeWidthStr::width(sep) as u16);

    layout_map.header_profile = Rect::new(cursor_x, area.y, env_w, 1);
    left_spans.push(Span::styled(
        env_badge,
        if state.profile.is_some() {
            theme::warning().add_modifier(Modifier::BOLD)
        } else {
            theme::warning()
        },
    ));
    cursor_x = cursor_x.saturating_add(env_w);

    let remaining = max_x.saturating_sub(cursor_x);
    if remaining > right_w {
        let pad = remaining.saturating_sub(right_w) as usize;
        left_spans.push(Span::raw(" ".repeat(pad)));
        left_spans.extend(right_spans);
    }

    frame.render_widget(Paragraph::new(Line::from(left_spans)), area);
}

fn build_header_right_spans(state: &AppState) -> Vec<Span<'static>> {
    let mut spans = Vec::new();

    if !state.loading_domains.is_empty() {
        let names: Vec<String> = state
            .loading_domains
            .iter()
            .take(2)
            .map(ToString::to_string)
            .collect();
        let extra = state.loading_domains.len().saturating_sub(2);
        let label = if extra > 0 {
            format!("[{} {}, +{}]", state.spinner_char(), names.join(","), extra)
        } else {
            format!("[{} {}]", state.spinner_char(), names.join(", "))
        };
        spans.push(Span::styled(label, theme::accent()));
        spans.push(Span::raw(" "));
    }

    if let Some(status_span) = build_header_job_status_span(state, false) {
        spans.push(status_span);
        spans.push(Span::raw(" "));
    }

    if state.screen_mode != ScreenMode::Normal {
        spans.push(Span::styled(
            format!("[ZOOM:{}]", state.screen_mode.label().to_ascii_uppercase()),
            theme::warning().add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::raw(" "));
    }

    spans.push(Span::styled("[+:zoom ?:help] ", theme::subtle()));
    spans
}

fn build_header_job_status_span(state: &AppState, compact: bool) -> Option<Span<'static>> {
    let running_count = state.running_jobs_count();
    let max_label = if compact { 12 } else { 20 };

    if running_count > 0 {
        let latest_running = state.jobs.values().rev().find(|j| j.status.is_running())?;
        let raw_name = latest_running
            .label
            .strip_prefix("task:")
            .unwrap_or(&latest_running.label);
        let name = truncate_display(raw_name, max_label);
        let elapsed = format_duration_ms(latest_running.started_at.elapsed().as_millis());
        let text = if running_count > 1 {
            format!(
                "[{} {} +{} {}]",
                state.spinner_char(),
                name,
                running_count - 1,
                elapsed
            )
        } else {
            format!("[{} {} {}]", state.spinner_char(), name, elapsed)
        };
        return Some(Span::styled(
            text,
            theme::accent().add_modifier(Modifier::BOLD),
        ));
    }

    let latest = state.jobs.values().next_back()?;
    let raw_name = latest
        .label
        .strip_prefix("task:")
        .unwrap_or(&latest.label);
    let name = truncate_display(raw_name, max_label);
    match latest.status {
        JobStatus::Success { duration_ms } => Some(Span::styled(
            format!(
                "[{} {} {}]",
                glyphs::OK,
                name,
                format_duration_ms(duration_ms)
            ),
            theme::success(),
        )),
        JobStatus::Failed { code, .. } => Some(Span::styled(
            format!("[{} {} exit {code}]", glyphs::FAIL, name),
            theme::danger(),
        )),
        JobStatus::Killed { .. } => Some(Span::styled(
            format!("[⊘ {name} killed]"),
            theme::warning(),
        )),
        JobStatus::Running => None,
    }
}

fn render_footer(frame: &mut Frame, state: &AppState, area: Rect) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let (panel_hints, global_hints) = footer_hint_groups_for_state(state);

    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut used = 0usize;
    let max_w = area.width as usize;

    // On very short terminals (< 12 rows), show active toast inline in footer instead of floating card.
    if frame.area().height < 12
        && let Some(toast) = state.active_toast()
    {
        let msg = truncate_display(&toast.message, (max_w / 2).max(12));
        let token = format!(" {msg} │ ");
        used += UnicodeWidthStr::width(token.as_str());
        spans.push(Span::styled(format!(" {msg} "), theme::toast(toast.level)));
        spans.push(Span::styled("│ ", theme::subtle()));
    } else {
        spans.push(Span::raw(" "));
        used += 1;
    }

    // Reserve room for essential global actions (`x:menu`, `?:help`, `q:quit`) on tighter viewports
    // so panel hints never push discovery/exit keys completely off-screen.
    let essential_global: Vec<(&'static str, &'static str)> = global_hints
        .iter()
        .copied()
        .filter(|(k, _)| matches!(*k, "x" | "?" | "q"))
        .collect();
    let essential_w: usize = essential_global
        .iter()
        .map(|(k, label)| UnicodeWidthStr::width(format!("{k}:{label}  ").as_str()))
        .sum::<usize>()
        + if panel_hints.is_empty() || essential_global.is_empty() {
            0
        } else {
            2 // "│ "
        };
    let panel_budget = max_w.saturating_sub(essential_w);

    let mut rendered_panel = 0usize;
    for (k, label) in &panel_hints {
        let piece = format!("{k}:{label}  ");
        let pw = UnicodeWidthStr::width(piece.as_str());
        if used + pw > panel_budget && rendered_panel > 0 {
            break;
        }
        if used + pw > max_w {
            break;
        }
        spans.push(Span::styled(*k, theme::label()));
        spans.push(Span::styled(format!(":{label}  "), theme::muted()));
        used += pw;
        rendered_panel += 1;
    }

    if rendered_panel > 0 && !global_hints.is_empty() && used + 2 <= max_w {
        spans.push(Span::styled("│ ", theme::subtle()));
        used += 2;
    }

    // Try rendering full global hints if they all fit; otherwise render essential global hints.
    let full_global_w: usize = global_hints
        .iter()
        .map(|(k, label)| UnicodeWidthStr::width(format!("{k}:{label}  ").as_str()))
        .sum();
    let globals_to_draw = if used + full_global_w <= max_w {
        &global_hints
    } else {
        &essential_global
    };

    for (k, label) in globals_to_draw {
        let piece = format!("{k}:{label}  ");
        let pw = UnicodeWidthStr::width(piece.as_str());
        if used + pw > max_w {
            break;
        }
        spans.push(Span::styled(*k, theme::label()));
        spans.push(Span::styled(format!(":{label}  "), theme::muted()));
        used += pw;
    }

    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

#[cfg(test)]
mod tests {
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::PathBuf;
    use std::time::{Duration, Instant, SystemTime};

    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use serde_json::json;

    use super::*;
    use crate::action::{
        Action, ConfigSubView, EnvSubView, LogPanelMode, TaskSubView, ToastLevel, ToolScope,
        ToolSubView,
    };
    use crate::app::state::{
        ArgField, ConfirmAction, GenerateItem, GenerateKind, MenuItem, Overlay, RegistryStep,
        TextInput,
    };
    use crate::events::{CommandSource, JobId, JobOutputLine, JobRecord, JobStatus};
    use crate::mise::schema::{
        AuditReport, ConfigExplainItem, ConfigFileEntry, DaemonEntry, DepEntry, DoctorReport,
        DoctorShell, EnvExtendedEntry, MiseTask, MiseToolEntry, MiseToolSource, OutdatedEntry,
        RegistryEntry, SettingEntry, TaskCacheConfig, TaskHistoryEntry, TaskHistorySummary,
        ToolDetail, TrustEntry, Vulnerability,
    };
    use crate::model::task_tree::build_task_rows;
    use crate::ui::layout::HitTarget;

    fn sample_rich_state() -> AppState {
        let mut state = AppState::new(
            PathBuf::from("/Users/dev/workspace/lazymise"),
            Some("dev".to_string()),
            Panel::Tasks,
        );

        let tasks = vec![
            MiseTask {
                name: "check".to_string(),
                description: "运行格式检查、测试与 Clippy 构建验证".to_string(),
                tags: vec!["ci".to_string()],
                cache: Some(TaskCacheConfig::Bool(true)),
                depends: vec![json!("test"), json!("lint")],
                run: vec!["#!/usr/bin/env bash\ncargo test --all-targets\ncargo clippy".to_string()],
                history: Some(TaskHistorySummary {
                    task: "check".to_string(),
                    runs: 14,
                    failures: 1,
                    median_ms: Some(820),
                    last_ms: Some(790),
                    last_status: Some("ok".to_string()),
                    last_started_at: Some("2026-11-26T12:00:00Z".to_string()),
                }),
                ..Default::default()
            },
            MiseTask {
                name: "exp:中文任务🚀".to_string(),
                description: "测试宽字符渲染与截断安全".to_string(),
                run: vec!["echo \"你好世界\"".to_string()],
                ..Default::default()
            },
        ];
        state.task_rows = build_task_rows(&tasks, &BTreeSet::new(), "", false);
        state.tasks = tasks;
        state.list_states.tasks.select(Some(1));
        state
            .task_deps_cache
            .insert("check".to_string(), vec!["check".to_string(), "├─ test".to_string(), "└─ lint".to_string()]);
        state.task_history_cache.insert(
            "check".to_string(),
            vec![TaskHistoryEntry {
                task: "check".to_string(),
                args: vec!["--all".to_string()],
                started_at: "2026-11-26 12:00:00".to_string(),
                duration_ms: 820,
                status: "ok".to_string(),
                exit_code: Some(0),
            }],
        );

        // Running background job with ANSI color output
        let job_id = JobId(1);
        state.jobs.insert(
            job_id,
            JobRecord {
                id: job_id,
                label: "check".to_string(),
                task_name: Some("check".to_string()),
                command_display: "mise run check".to_string(),
                pid: Some(4242),
                started_at: Instant::now(),
                status: JobStatus::Running,
                lines: vec![
                    JobOutputLine {
                        text: "\x1b[1;32m✓ Compiling\x1b[0m lazymise v0.2.0".to_string(),
                        is_stderr: false,
                    },
                    JobOutputLine {
                        text: "\x1b[33mwarning:\x1b[0m demo stderr line".to_string(),
                        is_stderr: true,
                    },
                ],
            },
        );
        state.latest_task_job.insert("check".to_string(), job_id);
        state.selected_job_id = Some(job_id);
        state.command_log.record(
            CommandSource::User,
            "mise run check",
            Some(job_id),
            JobStatus::Running,
            None,
        );
        state.command_log.record(
            CommandSource::Internal,
            "mise outdated --json",
            None,
            JobStatus::Success { duration_ms: 370 },
            Some("1 outdated".to_string()),
        );
        state.list_states.jobs.select(Some(0));

        // Tools VM
        let mut raw_tools = BTreeMap::new();
        raw_tools.insert(
            "node".to_string(),
            vec![MiseToolEntry {
                version: "22.1.0".to_string(),
                requested_version: Some("22".to_string()),
                install_path: Some("/Users/dev/.local/share/mise/installs/node/22.1.0".to_string()),
                source: Some(MiseToolSource {
                    kind: Some("mise.toml".to_string()),
                    path: Some("/Users/dev/workspace/lazymise/mise.toml".to_string()),
                }),
                installed: true,
                active: true,
                ..Default::default()
            }],
        );
        state.tools_vm.raw_tools = raw_tools;
        state.tools_vm.outdated.insert(
            "node".to_string(),
            OutdatedEntry {
                name: "node".to_string(),
                requested: Some("22".to_string()),
                current: Some("22.1.0".to_string()),
                latest: "23.0.0".to_string(),
                release_url: Some("https://nodejs.org".to_string()),
                ..Default::default()
            },
        );
        state.tools_vm.audit = Some(AuditReport {
            vulnerabilities: vec![Vulnerability {
                tool: "node".to_string(),
                version: "22.1.0".to_string(),
                id: "CVE-2026-1234".to_string(),
                severity: Some("HIGH".to_string()),
                summary: "Sample vulnerability".to_string(),
                fixed: vec!["22.12.0".to_string()],
                ..Default::default()
            }],
            unchecked: vec![],
        });
        state.tools_vm.details_cache.insert(
            "node".to_string(),
            ToolDetail {
                backend: Some("core:node".to_string()),
                description: Some("Node.js runtime".to_string()),
                url: Some("https://nodejs.org".to_string()),
                ..Default::default()
            },
        );
        state.tools_vm.lock_preview = Some("+ [tools.node]\n- version = \"22.0.0\"\n+ version = \"22.1.0\"".to_string());
        state.tools_vm.registry = vec![
            RegistryEntry {
                short: "ripgrep".to_string(),
                backends: vec!["aqua:BurntSushi/ripgrep".to_string()],
                bins: vec!["rg".to_string()],
                description: Some("Fast search tool".to_string()),
                aliases: vec!["rg".to_string()],
            },
            RegistryEntry {
                short: "fd".to_string(),
                backends: vec!["aqua:sharkdp/fd".to_string()],
                bins: vec!["fd".to_string()],
                description: Some("Fast find".to_string()),
                aliases: vec![],
            },
        ];
        state.tools_vm.rebuild(&state.workdir, ToolScope::All, "");
        state.list_states.tools.select(Some(0));

        // Env VM
        state.env_vm.raw_extended.insert(
            "API_TOKEN".to_string(),
            EnvExtendedEntry {
                value: "line1-secret\nline2-secret".to_string(),
                source: Some("/Users/dev/workspace/lazymise/mise.local.toml".to_string()),
                tool: None,
            },
        );
        state.env_vm.raw_extended.insert(
            "RUST_LOG".to_string(),
            EnvExtendedEntry {
                value: "info\ndebug".to_string(),
                source: Some("/Users/dev/workspace/lazymise/mise.toml".to_string()),
                tool: None,
            },
        );
        state.env_vm.paths = vec!["/usr/local/bin".to_string(), "/nonexistent/bin".to_string()];
        state
            .env_vm
            .vars
            .insert("app".to_string(), json!("lazymise"));
        state.env_vm.rebuild(&state.workdir, state.env_filter, "");
        state.list_states.env.select(Some(0));

        // Config VM
        state.config_vm.raw_files = vec![ConfigFileEntry {
            path: "/Users/dev/workspace/lazymise/mise.toml".to_string(),
            tools: vec!["rust".to_string()],
            is_override: false,
        }];
        state.config_vm.trust_entries = vec![TrustEntry {
            path: "/Users/dev/workspace/lazymise/mise.toml".to_string(),
            trusted: true,
            raw_status: "trusted".to_string(),
        }];
        state.config_vm.explain_items = vec![ConfigExplainItem {
            key: "legacy".to_string(),
            line: 2,
            deprecated: true,
            known: true,
            suggestion: Some("tools".to_string()),
            ..Default::default()
        }];
        state.config_vm.doctor = Some(DoctorReport {
            activated: true,
            version: "2026.11.13".to_string(),
            shell: Some(DoctorShell {
                name: "zsh".to_string(),
                version: "5.9".to_string(),
            }),
            warnings: vec!["Sample doctor warning".to_string()],
            errors: vec![],
            shims_on_path: true,
            ..Default::default()
        });
        state.config_vm.settings = vec![SettingEntry {
            key: "experimental".to_string(),
            value: json!(true),
            kind: "bool".to_string(),
            description: "Enable experimental features".to_string(),
            source: Some("~/.config/mise/config.toml".to_string()),
        }];
        state.config_vm.daemons = vec![DaemonEntry {
            name: "postgres".to_string(),
            status: "running".to_string(),
            pid: Some(9001),
            url: Some("postgres://localhost:5432".to_string()),
            ..Default::default()
        }];
        state.config_vm.deps = vec![DepEntry {
            name: "cargo".to_string(),
            status: "fresh".to_string(),
            detail: "Cargo.lock synced".to_string(),
        }];
        state.config_vm.rebuild("");
        state.list_states.config.select(Some(0));

        state.push_toast(ToastLevel::Success, "Loaded workspace cleanly");
        state
    }

    fn render_to_string(state: &mut AppState, width: u16, height: u16) -> (String, LayoutMap) {
        let backend = TestBackend::new(width, height);
        let mut terminal = Terminal::new(backend).unwrap();
        let mut layout = LayoutMap::default();
        terminal
            .draw(|f| render(f, state, &mut layout))
            .unwrap();
        let buf = terminal.backend().buffer();
        let mut text = String::new();
        for y in 0..height {
            let mut x = 0u16;
            while x < width {
                let sym = buf[(x, y)].symbol();
                text.push_str(sym);
                let step = UnicodeWidthStr::width(sym).max(1) as u16;
                x = x.saturating_add(step);
            }
            text.push('\n');
        }
        (text, layout)
    }

    #[test]
    fn renders_wide_140x40_all_panels_subviews_and_valid_layout_map() {
        let mut state = sample_rich_state();
        let (out, layout) = render_to_string(&mut state, 140, 40);

        assert!(!layout.is_narrow);
        assert!(out.contains("lazymise"));
        assert!(out.contains("ENV:[dev]"));
        assert!(out.contains("[1] TASKS"));
        assert!(out.contains("[2] TOOLS"));
        assert!(out.contains("[3] ENV"));
        assert!(out.contains("[4] CONFIG & HEALTH"));
        assert!(out.contains("[5] COMMAND LOG & JOBS"));
        assert!(out.contains("中文任务"));
        assert!(out.contains("Spec"));

        // Verify LayoutMap hit-testing on profile badge, 4 sidebar accordion cards, and 4 subtabs.
        assert_eq!(layout.sidebar_panels.len(), 4);
        assert_eq!(layout.main_subtabs.len(), 4);
        let prof = layout.header_profile;
        assert!(prof.width > 0);
        assert_eq!(
            layout.hit_test(prof.x + 1, prof.y, 0),
            Some(HitTarget::HeaderProfile)
        );
        let (_, tab1) = layout.main_subtabs[1];
        assert_eq!(
            layout.hit_test(tab1.x + 1, tab1.y, 0),
            Some(HitTarget::MainSubTab(1))
        );

        // Cycle every TaskSubView including ANSI Output
        for sub in TaskSubView::ALL {
            state.task_subview = sub;
            let (s, _) = render_to_string(&mut state, 140, 40);
            assert!(s.contains(sub.label()));
            if sub == TaskSubView::Output {
                assert!(s.contains("Compiling"));
            }
        }

        // Cycle Tools panel & all 4 subviews
        state.active_panel = Panel::Tools;
        for sub in ToolSubView::ALL {
            state.tool_subview = sub;
            let (s, _) = render_to_string(&mut state, 140, 40);
            assert!(s.contains(sub.label()));
        }

        // Cycle Env panel & all 3 subviews (plus revealed countdown)
        state.active_panel = Panel::Env;
        state
            .revealed_env_keys
            .insert("API_TOKEN".to_string(), Instant::now() + Duration::from_secs(20));
        for sub in EnvSubView::ALL {
            state.env_subview = sub;
            let (s, _) = render_to_string(&mut state, 140, 40);
            assert!(s.contains(sub.label()));
        }

        // Cycle Config panel & all 4 subviews
        state.active_panel = Panel::Config;
        for sub in ConfigSubView::ALL {
            state.config_subview = sub;
            let (s, _) = render_to_string(&mut state, 140, 40);
            assert!(s.contains(sub.label()));
        }

        // Jobs panel main output view
        state.active_panel = Panel::Jobs;
        state.focus = FocusArea::Main;
        let (s, _) = render_to_string(&mut state, 140, 40);
        assert!(s.contains("JOB OUTPUT"));
        assert!(s.contains("Compiling"));
    }

    #[test]
    fn renders_narrow_80x24_tiny_40x10_and_zoom_modes_without_panic() {
        let mut state = sample_rich_state();

        // Narrow 80x24
        let (narrow_out, narrow_layout) = render_to_string(&mut state, 80, 24);
        assert!(narrow_layout.is_narrow);
        assert!(narrow_out.contains("LIST ←→ detail"));
        assert_eq!(narrow_layout.header_panels.len(), 5);

        state.focus = FocusArea::Main;
        let (narrow_main, _) = render_to_string(&mut state, 80, 24);
        assert!(narrow_main.contains("list ←→ DETAIL"));

        // Tiny 40x10
        state.focus = FocusArea::Sidebar;
        let (tiny_out, tiny_layout) = render_to_string(&mut state, 40, 10);
        assert!(tiny_layout.is_narrow);
        assert!(tiny_out.contains("[1]"));

        // Half and Full zoom on wide screen
        state.screen_mode = ScreenMode::Half;
        state.log_mode = LogPanelMode::Expanded;
        let (half_out, _) = render_to_string(&mut state, 140, 40);
        assert!(half_out.contains("ZOOM:HALF"));

        state.screen_mode = ScreenMode::Full;
        for focus in [FocusArea::Sidebar, FocusArea::Main, FocusArea::Log] {
            state.focus = focus;
            let (full_out, _) = render_to_string(&mut state, 140, 40);
            assert!(full_out.contains("ZOOM:FULL"));
        }
    }

    #[test]
    fn renders_all_overlays_across_wide_and_tiny_viewports() {
        let overlays: Vec<Overlay> = vec![
            Overlay::Help {
                query: TextInput::new("run"),
                selected: 0,
            },
            Overlay::Filter {
                panel: Panel::Tasks,
                input: TextInput::new("中文"),
            },
            Overlay::ContextMenu {
                title: "Task Actions".to_string(),
                items: vec![MenuItem {
                    key_hint: "b".to_string(),
                    label: "Run background".to_string(),
                    description: "Stream live ANSI output".to_string(),
                    action: Action::TaskRunBackground,
                    command_preview: Some("mise run check".to_string()),
                }],
                selected: 0,
            },
            Overlay::CommandPalette {
                query: TextInput::new("help"),
                selected: 0,
            },
            Overlay::ProfileSwitcher {
                candidates: vec!["dev".to_string(), "prod".to_string()],
                input: TextInput::new("dev"),
                selected: 0,
            },
            Overlay::Confirm {
                title: "Prune unused tools".to_string(),
                detail: "This removes 2 unused tool versions.".to_string(),
                command_preview: "mise prune".to_string(),
                action: ConfirmAction::MiseCommand {
                    args: vec!["prune".to_string()],
                    foreground: true,
                    reload_after: true,
                    label: "prune".to_string(),
                },
            },
            Overlay::TaskArgsForm {
                task_name: "deploy".to_string(),
                background: true,
                force: true,
                fields: vec![ArgField {
                    name: "env".to_string(),
                    flag: Some("--env".to_string()),
                    help: "Target environment".to_string(),
                    required: true,
                    input: TextInput::new("staging"),
                }],
                extra_args: TextInput::new("--dry-run"),
                focused_field: 0,
            },
            Overlay::RegistryPicker {
                query: TextInput::new("rg"),
                selected: 0,
                step: RegistryStep::SearchTool,
            },
            Overlay::RegistryPicker {
                query: TextInput::default(),
                selected: 0,
                step: RegistryStep::PickVersion {
                    tool: "ripgrep".to_string(),
                    versions: vec!["14.1.1".to_string()],
                    loading: false,
                },
            },
            Overlay::RegistryPicker {
                query: TextInput::default(),
                selected: 0,
                step: RegistryStep::PickScope {
                    tool: "ripgrep".to_string(),
                    version: "latest".to_string(),
                },
            },
            Overlay::EnvSetForm {
                key: TextInput::new("API_SECRET"),
                value: TextInput::new("super-secret"),
                target_files: vec!["mise.toml".to_string(), "mise.local.toml".to_string()],
                target_idx: 1,
                focused_field: 1,
                is_edit: false,
            },
            Overlay::GenerateMenu {
                items: vec![GenerateItem {
                    label: "mise generate config".to_string(),
                    description: "Starter mise.toml".to_string(),
                    command_preview: "mise generate config".to_string(),
                    kind: GenerateKind::MiseGenerate {
                        args: vec!["generate".to_string(), "config".to_string()],
                    },
                }],
                selected: 0,
                template_name_input: None,
            },
            Overlay::GenerateMenu {
                items: vec![],
                selected: 0,
                template_name_input: Some(("rust-cli".to_string(), TextInput::new("my-app"))),
            },
        ];

        for ov in overlays {
            let mut state = sample_rich_state();
            state.overlay = ov;

            let (_, wide_layout) = render_to_string(&mut state, 140, 40);
            let ov_rect = wide_layout.overlay_area.expect("overlay rect must be registered");
            assert!(ov_rect.width > 0 && ov_rect.height > 0);
            assert_eq!(
                wide_layout.hit_test(ov_rect.x, ov_rect.y, 0),
                Some(HitTarget::Overlay)
            );

            // Also verify zero-panic rendering on tiny 40x10 terminal
            let _ = render_to_string(&mut state, 40, 10);
        }

        // Verify SystemTime formatting stability
        assert_eq!(widgets::format_hms(SystemTime::UNIX_EPOCH), "00:00:00");
    }

    #[test]
    fn renders_top_right_header_job_status_for_running_and_completed_states() {
        let mut state = sample_rich_state();

        // 1. Running single job -> shows spinner + label in wide & narrow headers
        let (wide_run, _) = render_to_string(&mut state, 140, 40);
        let header_line = wide_run.lines().next().unwrap_or("");
        assert!(header_line.contains("[⠋ check "));

        let (narrow_run, _) = render_to_string(&mut state, 80, 24);
        let narrow_header = narrow_run.lines().next().unwrap_or("");
        assert!(narrow_header.contains("[⠋ check "));

        // 2. Multiple running jobs -> shows "+1" counter
        state.jobs.insert(
            JobId(2),
            JobRecord {
                id: JobId(2),
                label: "task:build".to_string(),
                task_name: Some("build".to_string()),
                command_display: "mise run build".to_string(),
                pid: None,
                started_at: Instant::now(),
                status: JobStatus::Running,
                lines: vec![],
            },
        );
        let (wide_multi, _) = render_to_string(&mut state, 140, 40);
        let multi_header = wide_multi.lines().next().unwrap_or("");
        assert!(multi_header.contains("[⠋ build +1 "));

        // 3. Completed success -> shows check mark + duration
        state.jobs.get_mut(&JobId(1)).unwrap().status = JobStatus::Success { duration_ms: 820 };
        state.jobs.get_mut(&JobId(2)).unwrap().status = JobStatus::Success { duration_ms: 1250 };
        let (wide_ok, _) = render_to_string(&mut state, 140, 40);
        let ok_header = wide_ok.lines().next().unwrap_or("");
        assert!(ok_header.contains("[✓ build 1.25s]"));

        // 4. Failed job -> shows failure glyph + exit code
        state.jobs.get_mut(&JobId(2)).unwrap().status = JobStatus::Failed {
            code: 2,
            duration_ms: 300,
        };
        let (wide_fail, _) = render_to_string(&mut state, 140, 40);
        let fail_header = wide_fail.lines().next().unwrap_or("");
        assert!(fail_header.contains("[✗ build exit 2]"));
    }
}
