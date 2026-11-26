use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::action::{Domain, FocusArea, Panel, TaskSubView};
use crate::app::state::AppState;
use crate::events::JobStatus;
use crate::mise::schema::MiseTask;
use crate::model::task_tree::{TaskRow, task_leaf_name};
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};
use crate::ui::widgets::{
    empty_state_lines, format_duration_ms, highlight_shell_line, highlighted_truncated_spans,
    kv_line, main_subtab_block, panel_block, render_job_output_line, render_scrollbar,
    section_divider, shorten_path, truncate_display,
};

/// Render the collapsed accordion summary card (1-line or 3-line) for `[1] TASKS`.
pub fn render_summary_card(
    frame: &mut Frame,
    state: &AppState,
    area: Rect,
    one_line: bool,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }
    layout.register_sidebar_panel(Panel::Tasks, area, Rect::default());

    let total = state.tasks.len();
    let running = running_task_count(state);
    let loading = state.loading_domains.contains(&Domain::Tasks);

    if one_line || area.height < 3 {
        let mut spans = vec![
            Span::styled("─ ", theme::subtle()),
            Span::styled("[1] TASKS ", theme::panel_title(false)),
            Span::styled(format!("({total}"), theme::muted()),
        ];
        if running > 0 {
            spans.push(Span::styled(
                format!(" {} {} {running} run", glyphs::BULLET, state.spinner_char()),
                theme::accent(),
            ));
        } else if loading {
            spans.push(Span::styled(
                format!(" {} {}", glyphs::BULLET, state.spinner_char()),
                theme::accent(),
            ));
        }
        spans.push(Span::styled(") ", theme::muted()));
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let mut title_spans = vec![
        Span::styled(" [1] TASKS ", theme::panel_title(false)),
        Span::styled(format!("({total}"), theme::muted()),
    ];
    if running > 0 {
        title_spans.push(Span::styled(
            format!(" {} {} {running} run", glyphs::BULLET, state.spinner_char()),
            theme::accent(),
        ));
    }
    title_spans.push(Span::styled(") ", theme::muted()));

    let block = panel_block(Line::from(title_spans), None, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let summary_line = if loading && state.tasks.is_empty() {
        Line::from(Span::styled(
            format!("{} Loading tasks…", state.spinner_char()),
            theme::muted(),
        ))
    } else if state.tasks.is_empty() {
        Line::from(Span::styled("No tasks defined", theme::muted()))
    } else {
        let mut spans = Vec::new();
        let mut used = 0usize;
        let max_w = inner.width as usize;
        for task in state.tasks.iter().filter(|t| !t.hide).take(4) {
            let (glyph, gstyle) = task_status_glyph(state, task);
            let piece = format!("{glyph} {}  ", task_leaf_name(task));
            let pw = UnicodeWidthStr::width(piece.as_str());
            if used + pw > max_w && !spans.is_empty() {
                break;
            }
            spans.push(Span::styled(format!("{glyph} "), gstyle));
            spans.push(Span::styled(
                format!("{}  ", truncate_display(task_leaf_name(task), max_w.saturating_sub(used + 3))),
                theme::primary(),
            ));
            used += pw;
        }
        if spans.is_empty() {
            Line::from(Span::styled(format!("{total} tasks"), theme::muted()))
        } else {
            Line::from(spans)
        }
    };

    frame.render_widget(Paragraph::new(summary_line), inner);
}

/// Render the expanded `[1] TASKS` accordion sidebar with collapsible tree and status badges.
pub fn render_sidebar(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Tasks && state.focus == FocusArea::Sidebar;
    let total = state.tasks.len();
    let running = running_task_count(state);
    let loading = state.loading_domains.contains(&Domain::Tasks);

    let mut title_spans = vec![
        Span::styled(" [1] TASKS ", theme::panel_title(focused)),
        Span::styled(format!("({total}"), theme::muted()),
    ];
    if running > 0 {
        title_spans.push(Span::styled(
            format!(" {} {} {running} run", glyphs::BULLET, state.spinner_char()),
            theme::accent(),
        ));
    } else if loading {
        title_spans.push(Span::styled(
            format!(" {} {}", glyphs::BULLET, state.spinner_char()),
            theme::accent(),
        ));
    }
    title_spans.push(Span::styled(") ", theme::muted()));

    let mut right_spans = Vec::new();
    if state.show_hidden_tasks {
        right_spans.push(Span::styled(" +hid", theme::purple()));
    }
    if state.monorepo_all_tasks {
        right_spans.push(Span::styled(" +mono", theme::purple()));
    }
    if !state.filters.tasks.is_empty() {
        right_spans.push(Span::styled(
            format!(" /{}", truncate_display(&state.filters.tasks, 12)),
            theme::highlight_match(),
        ));
    }
    let right_title = if right_spans.is_empty() {
        None
    } else {
        right_spans.push(Span::styled(" ", theme::muted()));
        Some(Line::from(right_spans))
    };

    let block = panel_block(Line::from(title_spans), right_title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.register_sidebar_panel(Panel::Tasks, area, inner);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if state.task_rows.is_empty() {
        let msg = if loading {
            format!("{} Loading tasks from mise…", state.spinner_char())
        } else if !state.filters.tasks.is_empty() {
            format!("No tasks match '/{}' (Esc to clear)", state.filters.tasks)
        } else {
            "No mise tasks found in this workspace.".to_string()
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme::muted())))
                .wrap(Wrap { trim: true }),
            inner,
        );
        return;
    }

    let inner_w = inner.width as usize;
    let items: Vec<ListItem<'static>> = state
        .task_rows
        .iter()
        .map(|row| match row {
            TaskRow::Group {
                key,
                count,
                collapsed,
            } => {
                let arrow = if *collapsed {
                    glyphs::FOLD_CLOSED
                } else {
                    glyphs::FOLD_OPEN
                };
                let key_style = if key == "[root]" {
                    theme::label()
                } else if key == "[global]" {
                    theme::muted().add_modifier(Modifier::BOLD)
                } else if key.starts_with("//") {
                    theme::purple().add_modifier(Modifier::BOLD)
                } else {
                    theme::section_header()
                };
                let count_str = format!(" ({count})");
                let name_budget = inner_w.saturating_sub(UnicodeWidthStr::width(count_str.as_str()) + 3);
                ListItem::new(Line::from(vec![
                    Span::styled(format!("{arrow} "), theme::muted()),
                    Span::styled(truncate_display(key, name_budget), key_style),
                    Span::styled(count_str, theme::muted()),
                ]))
            }
            TaskRow::Task {
                task,
                match_indices,
            } => {
                let (glyph, glyph_style) = task_status_glyph(state, task);
                let mut suffix_spans: Vec<Span<'static>> = Vec::new();
                let mut suffix_w = 0usize;

                if task.cache_enabled() {
                    let s = format!(" {}", glyphs::CACHE);
                    suffix_w += UnicodeWidthStr::width(s.as_str());
                    suffix_spans.push(Span::styled(s, theme::warning()));
                }
                if let Some(tag) = task.tags.first() {
                    let s = format!(" #{tag}");
                    let sw = UnicodeWidthStr::width(s.as_str());
                    if inner_w > 28 && suffix_w + sw < inner_w / 2 {
                        suffix_w += sw;
                        suffix_spans.push(Span::styled(s, theme::purple()));
                    }
                }
                if let Some(dur) = task_duration_badge(state, task) {
                    let s = format!(" {dur}");
                    let sw = UnicodeWidthStr::width(s.as_str());
                    if inner_w > 22 && suffix_w + sw < inner_w / 2 {
                        suffix_w += sw;
                        suffix_spans.push(Span::styled(s, theme::muted()));
                    }
                }
                if task.hide {
                    let s = " (h)";
                    suffix_w += s.len();
                    suffix_spans.push(Span::styled(s, theme::subtle()));
                }

                let prefix_w = 4; // "  ✓ "
                let name_budget = inner_w.saturating_sub(prefix_w + suffix_w).max(4);
                let base_style = if task.hide {
                    theme::muted()
                } else {
                    theme::primary()
                };

                let mut spans = vec![
                    Span::raw("  "),
                    Span::styled(format!("{glyph} "), glyph_style),
                ];
                spans.extend(highlighted_truncated_spans(
                    task_leaf_name(task),
                    match_indices,
                    base_style,
                    name_budget,
                ));
                spans.extend(suffix_spans);
                ListItem::new(Line::from(spans))
            }
        })
        .collect();

    let total_rows = items.len();
    let list = List::new(items).highlight_style(theme::selected(focused));
    frame.render_stateful_widget(list, inner, &mut state.list_states.tasks);

    render_scrollbar(
        frame,
        area,
        total_rows,
        inner.height as usize,
        state.list_states.tasks.offset(),
        focused,
    );
}

/// Render the right multi-subview main pane for `[1] TASKS` (`Spec | Output | Deps | History`).
pub fn render_main(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Tasks && state.focus == FocusArea::Main;
    let labels = TaskSubView::ALL.map(TaskSubView::label);
    let active_idx = TaskSubView::ALL
        .iter()
        .position(|s| *s == state.task_subview)
        .unwrap_or(0);

    let right_hint = if state.task_subview == TaskSubView::Output {
        Some(if state.auto_follow {
            "FOLLOW:ON  [ / ]"
        } else {
            "FOLLOW:OFF [ / ]"
        })
    } else {
        None
    };

    let block = main_subtab_block(area, &labels, active_idx, focused, right_hint, layout);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.main_inner = inner;

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    state.detail_scroll.viewport_height = inner.height;

    let lines = match state.task_subview {
        TaskSubView::Spec => build_spec_lines(state, inner.width as usize),
        TaskSubView::Output => build_output_lines(state, inner.width as usize),
        TaskSubView::Deps => build_deps_lines(state, inner.width as usize),
        TaskSubView::History => build_history_lines(state, inner.width as usize),
    };

    let total_lines = lines.len().min(u16::MAX as usize) as u16;
    state.detail_scroll.content_len = total_lines;
    if state.task_subview == TaskSubView::Output && state.auto_follow {
        state.detail_scroll.scroll_bottom();
    } else {
        state.detail_scroll.clamp();
    }

    let scroll_y = state.detail_scroll.offset;
    let para = Paragraph::new(lines)
        .scroll((scroll_y, 0))
        .wrap(Wrap { trim: false });
    frame.render_widget(para, inner);

    render_scrollbar(
        frame,
        area,
        total_lines as usize,
        inner.height as usize,
        scroll_y as usize,
        focused,
    );
}

fn build_spec_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(task) = state.selected_task() else {
        if let Some(idx) = state.list_states.tasks.selected()
            && let Some(TaskRow::Group {
                key,
                count,
                collapsed,
            }) = state.task_rows.get(idx)
        {
            return empty_state_lines(
                &format!("Task Group: {key}"),
                &format!(
                    "{count} tasks ({})",
                    if *collapsed { "collapsed" } else { "expanded" }
                ),
                Some("Press Space / Enter / za to toggle fold, or j/k to select a task."),
            );
        }
        return empty_state_lines(
            "No Task Selected",
            "Select a task from the left sidebar to inspect its specification and script.",
            Some("Press Enter to run foreground, b for background, A for arguments, or x for menu."),
        );
    };

    // Merge richer info from `task_info_cache` if loaded.
    let task = state.task_info_cache.get(&task.name).unwrap_or(task);

    let mut lines = Vec::new();
    let scope = if task.global {
        "global"
    } else if task.name.starts_with("//") {
        "monorepo"
    } else {
        "workspace"
    };

    lines.push(kv_line("NAME", task.name.clone(), theme::label()));
    if !task.description.is_empty() {
        lines.push(kv_line("DESC", task.description.clone(), theme::primary()));
    }

    let mut meta_spans = vec![
        Span::styled(format!("{:<10} ", "SCOPE"), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(scope, theme::accent()),
    ];
    if !task.tags.is_empty() {
        meta_spans.push(Span::styled("   TAGS: ", theme::muted()));
        meta_spans.push(Span::styled(task.tags.join(", "), theme::purple()));
    }
    meta_spans.push(Span::styled("   CACHE: ", theme::muted()));
    if task.cache_enabled() {
        meta_spans.push(Span::styled(format!("{} on", glyphs::CACHE), theme::success()));
    } else {
        meta_spans.push(Span::styled("off", theme::muted()));
    }
    lines.push(Line::from(meta_spans));

    if !task.aliases.is_empty() {
        lines.push(kv_line("ALIASES", task.aliases.join(", "), theme::primary()));
    }
    if let Some(path) = task.editable_path() {
        lines.push(kv_line(
            "SOURCE",
            shorten_path(path, width.saturating_sub(12)),
            theme::muted(),
        ));
    }
    if let Some(dir) = task.dir.as_deref().filter(|d| !d.is_empty()) {
        lines.push(kv_line("DIR", dir.to_string(), theme::primary()));
    }

    let deps = task.depends_display();
    if !deps.is_empty() {
        lines.push(kv_line("DEPENDS", deps.join(", "), theme::accent()));
    }
    let deps_post = task.depends_post_display();
    if !deps_post.is_empty() {
        lines.push(kv_line("POST", deps_post.join(", "), theme::accent()));
    }
    let wait_for = task.wait_for_display();
    if !wait_for.is_empty() {
        lines.push(kv_line("WAIT FOR", wait_for.join(", "), theme::muted()));
    }
    if !task.tools.is_empty() {
        let tools_str = task
            .tools
            .iter()
            .map(|(k, v)| format!("{k}@{v}"))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(kv_line("TOOLS", tools_str, theme::success()));
    }
    if !task.sources.is_empty() {
        lines.push(kv_line("SOURCES", task.sources.join(", "), theme::muted()));
    }
    let outputs = task.outputs_display();
    if !outputs.is_empty() {
        lines.push(kv_line("OUTPUTS", outputs.join(", "), theme::muted()));
    }
    if let Some(timeout) = task.timeout_display() {
        lines.push(kv_line("TIMEOUT", timeout, theme::warning()));
    }

    let mut flags = Vec::new();
    if task.raw {
        flags.push("raw");
    }
    if task.interactive {
        flags.push("interactive");
    }
    if task.quiet {
        flags.push("quiet");
    }
    if task.is_silent() {
        flags.push("silent");
    }
    if task.hide {
        flags.push("hidden");
    }
    if task.has_daemons() {
        flags.push("daemons");
    }
    if !flags.is_empty() {
        lines.push(kv_line("FLAGS", flags.join(" • "), theme::warning()));
    }

    if let Some(hist) = &task.history {
        let mut h_parts = vec![format!("{} runs", hist.runs)];
        if hist.failures > 0 {
            h_parts.push(format!("{} failures", hist.failures));
        }
        if let Some(med) = hist.median_ms {
            h_parts.push(format!("median {}", format_duration_ms(med as u128)));
        }
        if let Some(last) = hist.last_status.as_deref() {
            h_parts.push(format!("last: {last}"));
        }
        lines.push(kv_line("HISTORY", h_parts.join(" • "), theme::muted()));
    }

    if let Some(spec) = &task.usage_spec
        && spec.has_any_inputs()
    {
        let mut usage_parts = Vec::new();
        for arg in &spec.cmd.args {
            if arg.required {
                usage_parts.push(format!("<{}>", arg.name));
            } else {
                usage_parts.push(format!("[{}]", arg.name));
            }
        }
        for flag in &spec.cmd.flags {
            let label = flag
                .long
                .first()
                .map(|l| format!("--{l}"))
                .or_else(|| flag.short.first().map(|s| format!("-{s}")))
                .unwrap_or_else(|| flag.name.clone());
            usage_parts.push(label);
        }
        lines.push(kv_line("USAGE", usage_parts.join(" "), theme::purple()));
    } else if !task.usage.trim().is_empty() {
        lines.push(kv_line(
            "USAGE",
            task.usage.lines().next().unwrap_or("").trim().to_string(),
            theme::purple(),
        ));
    }

    lines.push(Line::from(""));
    lines.push(section_divider("SCRIPT / COMMAND (cached)", width));

    // Zero disk I/O: strictly read from `state.script_cache`.
    let script = state.script_cache.task_script_readonly(task);
    for (idx, raw_line) in script.lines().enumerate() {
        let num = format!("{:>3} │ ", idx + 1);
        let highlighted = highlight_shell_line(raw_line);
        let mut spans = vec![Span::styled(num, theme::subtle())];
        spans.extend(highlighted.spans);
        lines.push(Line::from(spans));
    }

    lines
}

fn build_output_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(task) = state.selected_task() else {
        return empty_state_lines(
            "No Task Selected",
            "Select a task and press `b` to run it in the background with live ANSI output.",
            None,
        );
    };

    let Some(job) = state.selected_task_job() else {
        return empty_state_lines(
            &format!("No Background Output for `{}`", task.name),
            "This task has not been run in the background during this session.",
            Some("Press `b` to run in background (live ANSI stream), or `Enter` for foreground TTY."),
        );
    };

    let mut lines = Vec::new();
    let (status_str, status_style, dur_str) = match job.status {
        JobStatus::Running => (
            format!("{} RUNNING", state.spinner_char()),
            theme::accent().add_modifier(Modifier::BOLD),
            format_duration_ms(job.started_at.elapsed().as_millis()),
        ),
        JobStatus::Success { duration_ms } => (
            format!("{} EXIT 0", glyphs::OK),
            theme::success().add_modifier(Modifier::BOLD),
            format_duration_ms(duration_ms),
        ),
        JobStatus::Failed { code, duration_ms } => (
            format!("{} EXIT {code}", glyphs::FAIL),
            theme::danger().add_modifier(Modifier::BOLD),
            format_duration_ms(duration_ms),
        ),
        JobStatus::Killed { duration_ms } => (
            format!("{} KILLED", glyphs::PRUNE),
            theme::warning().add_modifier(Modifier::BOLD),
            format_duration_ms(duration_ms),
        ),
    };

    lines.push(Line::from(vec![
        Span::styled(status_str, status_style),
        Span::styled("  ", theme::muted()),
        Span::styled(job.command_display.clone(), theme::primary()),
        Span::styled(format!("  [{dur_str}]"), theme::muted()),
    ]));
    lines.push(section_divider(
        &format!(
            "OUTPUT ({} lines • {})",
            job.lines.len(),
            if state.auto_follow {
                "auto-follow"
            } else {
                "paused"
            }
        ),
        width,
    ));

    if job.lines.is_empty() {
        lines.push(Line::from(Span::styled(
            if job.status.is_running() {
                "Waiting for output…"
            } else {
                "(no output produced)"
            },
            theme::muted(),
        )));
    } else {
        for out in &job.lines {
            let base = if out.is_stderr {
                theme::warning()
            } else {
                theme::primary()
            };
            lines.push(render_job_output_line(&out.text, base));
        }
    }

    lines
}

fn build_deps_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(task) = state.selected_task() else {
        return empty_state_lines(
            "Task Dependency Tree",
            "Select a task to inspect its upstream and downstream task graph.",
            None,
        );
    };

    let mut lines = vec![
        kv_line("TASK", task.name.clone(), theme::label()),
        Line::from(""),
        section_divider("DEPENDENCY GRAPH", width),
    ];

    if state.loading_domains.contains(&Domain::TaskDeps)
        && !state.task_deps_cache.contains_key(&task.name)
    {
        lines.push(Line::from(Span::styled(
            format!("  {} Loading `mise tasks deps {}`…", state.spinner_char(), task.name),
            theme::accent(),
        )));
        return lines;
    }

    if let Some(tree_lines) = state.task_deps_cache.get(&task.name)
        && !tree_lines.is_empty()
    {
        for (i, raw) in tree_lines.iter().enumerate() {
            let mut spans = Vec::new();
            let mut prefix = String::new();
            let mut rest = String::new();
            let mut seen_text = false;
            for ch in raw.chars() {
                if !seen_text && matches!(ch, ' ' | '│' | '├' | '└' | '─' | '|' | '+' | '-') {
                    prefix.push(ch);
                } else {
                    seen_text = true;
                    rest.push(ch);
                }
            }
            if !prefix.is_empty() {
                spans.push(Span::styled(prefix, theme::subtle()));
            }
            let text_style = if i == 0 {
                theme::label()
            } else {
                theme::primary()
            };
            spans.push(Span::styled(rest, text_style));
            lines.push(Line::from(spans));
        }
        return lines;
    }

    let deps = task.depends_display();
    let post = task.depends_post_display();
    let wait = task.wait_for_display();
    if deps.is_empty() && post.is_empty() && wait.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No dependencies configured for this task.",
            theme::muted(),
        )));
    } else {
        for dep in deps {
            lines.push(Line::from(vec![
                Span::styled("  ├─ depends: ", theme::subtle()),
                Span::styled(dep, theme::accent()),
            ]));
        }
        for dep in wait {
            lines.push(Line::from(vec![
                Span::styled("  ├─ wait_for: ", theme::subtle()),
                Span::styled(dep, theme::warning()),
            ]));
        }
        for dep in post {
            lines.push(Line::from(vec![
                Span::styled("  └─ depends_post: ", theme::subtle()),
                Span::styled(dep, theme::purple()),
            ]));
        }
    }

    lines
}

fn build_history_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(task) = state.selected_task() else {
        return empty_state_lines(
            "Task Run History",
            "Select a task to view execution history and median runtimes.",
            None,
        );
    };

    let mut lines = vec![kv_line("TASK", task.name.clone(), theme::label())];

    if let Some(summary) = &task.history {
        let mut parts = vec![format!("Runs: {}", summary.runs)];
        parts.push(format!("Failures: {}", summary.failures));
        if let Some(med) = summary.median_ms {
            parts.push(format!("Median: {}", format_duration_ms(med as u128)));
        }
        if let Some(last) = summary.last_ms {
            parts.push(format!("Last: {}", format_duration_ms(last as u128)));
        }
        lines.push(kv_line("SUMMARY", parts.join("  •  "), theme::primary()));
    }

    lines.push(Line::from(""));
    lines.push(section_divider("RECENT EXECUTIONS", width));

    if state.loading_domains.contains(&Domain::TaskHistory)
        && !state.task_history_cache.contains_key(&task.name)
    {
        lines.push(Line::from(Span::styled(
            format!("  {} Loading `mise tasks history`…", state.spinner_char()),
            theme::accent(),
        )));
        return lines;
    }

    let Some(entries) = state.task_history_cache.get(&task.name) else {
        lines.push(Line::from(Span::styled(
            "  No detailed history records loaded yet.",
            theme::muted(),
        )));
        return lines;
    };

    if entries.is_empty() {
        lines.push(Line::from(Span::styled(
            "  No executions recorded in mise task history.",
            theme::muted(),
        )));
        return lines;
    }

    lines.push(Line::from(vec![
        Span::styled(format!("  {:<8} ", "STATUS"), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<20} ", "STARTED"), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{:<10} ", "DURATION"), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled("ARGS", theme::muted().add_modifier(Modifier::BOLD)),
    ]));

    for entry in entries {
        let is_ok = entry.exit_code == Some(0)
            || entry.status.eq_ignore_ascii_case("success")
            || entry.status.eq_ignore_ascii_case("ok");
        let (badge, bstyle) = if is_ok {
            (format!("{} ok", glyphs::OK), theme::success())
        } else {
            let code = entry
                .exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| entry.status.clone());
            (format!("{} {code}", glyphs::FAIL), theme::danger())
        };

        let started = truncate_display(&entry.started_at, 20);
        let dur = format_duration_ms(entry.duration_ms as u128);
        let args = if entry.args.is_empty() {
            "—".to_string()
        } else {
            entry.args.join(" ")
        };

        lines.push(Line::from(vec![
            Span::styled(format!("  {:<8} ", badge), bstyle),
            Span::styled(format!("{:<20} ", started), theme::muted()),
            Span::styled(format!("{:<10} ", dur), theme::primary()),
            Span::styled(args, theme::accent()),
        ]));
    }

    lines
}

fn running_task_count(state: &AppState) -> usize {
    state
        .jobs
        .values()
        .filter(|j| j.status.is_running() && j.task_name.is_some())
        .count()
}

fn task_status_glyph(state: &AppState, task: &MiseTask) -> (String, ratatui::style::Style) {
    if let Some(job_id) = state.latest_task_job.get(&task.name)
        && let Some(job) = state.jobs.get(job_id)
    {
        return match job.status {
            JobStatus::Running => (state.spinner_char().to_string(), theme::accent()),
            JobStatus::Success { .. } => (glyphs::OK.to_string(), theme::success()),
            JobStatus::Failed { .. } => (glyphs::FAIL.to_string(), theme::danger()),
            JobStatus::Killed { .. } => (glyphs::PRUNE.to_string(), theme::warning()),
        };
    }

    if let Some(hist) = &task.history
        && let Some(last) = hist.last_status.as_deref()
    {
        let lower = last.to_ascii_lowercase();
        if lower == "ok" || lower == "success" || lower == "passed" || lower == "0" {
            return (glyphs::OK.to_string(), theme::success());
        }
        if lower.contains("fail") || lower.contains("err") {
            return (glyphs::FAIL.to_string(), theme::danger());
        }
    }

    (glyphs::IDLE.to_string(), theme::muted())
}

fn task_duration_badge(state: &AppState, task: &MiseTask) -> Option<String> {
    if let Some(job_id) = state.latest_task_job.get(&task.name)
        && let Some(job) = state.jobs.get(job_id)
    {
        let ms = match job.status {
            JobStatus::Running => job.started_at.elapsed().as_millis(),
            JobStatus::Success { duration_ms }
            | JobStatus::Failed { duration_ms, .. }
            | JobStatus::Killed { duration_ms } => duration_ms,
        };
        return Some(format!("[{}]", format_duration_ms(ms)));
    }

    if let Some(hist) = &task.history
        && let Some(ms) = hist.median_ms.or(hist.last_ms)
    {
        return Some(format!("[{}]", format_duration_ms(ms as u128)));
    }
    None
}
