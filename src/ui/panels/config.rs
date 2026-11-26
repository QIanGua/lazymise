use std::collections::HashMap;

use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::action::{ConfigSubView, Domain, FocusArea, Panel};
use crate::app::state::AppState;
use crate::mise::schema::ConfigExplainItem;
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};
use crate::ui::widgets::{
    empty_state_lines, highlight_toml_line, highlighted_truncated_spans, kv_line, main_subtab_block,
    pad_display_width, panel_block, render_scrollbar, section_divider, shorten_path,
    truncate_display,
};

/// Render the collapsed accordion summary card (1-line or 3-line) for `[4] CONFIG & HEALTH`.
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
    layout.register_sidebar_panel(Panel::Config, area, Rect::default());

    let stats = state.config_vm.stats();
    let loading = state.loading_domains.contains(&Domain::Config)
        || state.loading_domains.contains(&Domain::Doctor);

    let mut badge_spans = vec![Span::styled(format!("({}", stats.total_files), theme::muted())];
    if stats.doctor_errors > 0 {
        badge_spans.push(Span::styled(
            format!(" {} {}{}E", glyphs::BULLET, glyphs::FAIL, stats.doctor_errors),
            theme::danger(),
        ));
    }
    if stats.doctor_warnings > 0 {
        badge_spans.push(Span::styled(
            format!(" {} {}{}W", glyphs::BULLET, glyphs::WARN, stats.doctor_warnings),
            theme::warning(),
        ));
    } else if stats.doctor_errors == 0 && state.config_vm.doctor.is_some() {
        badge_spans.push(Span::styled(
            format!(" {} {}", glyphs::BULLET, glyphs::OK),
            theme::success(),
        ));
    }
    if loading {
        badge_spans.push(Span::styled(
            format!(" {} {}", glyphs::BULLET, state.spinner_char()),
            theme::accent(),
        ));
    }
    badge_spans.push(Span::styled(") ", theme::muted()));

    if one_line || area.height < 3 {
        let mut spans = vec![
            Span::styled("─ ", theme::subtle()),
            Span::styled("[4] CONFIG ", theme::panel_title(false)),
        ];
        spans.extend(badge_spans);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let mut title_spans = vec![Span::styled(" [4] CONFIG & HEALTH ", theme::panel_title(false))];
    title_spans.extend(badge_spans);
    let block = panel_block(Line::from(title_spans), None, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let line = if let Some(first) = state.config_vm.rows.first() {
        let fname = std::path::Path::new(&first.entry.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&first.entry.path);
        let (tg, ts) = trust_glyph(first.trusted);
        let mut spans = vec![
            Span::styled(format!("{tg} "), ts),
            Span::styled(
                truncate_display(fname, (inner.width as usize).saturating_sub(18).max(8)),
                theme::primary(),
            ),
        ];
        if stats.doctor_errors > 0 || stats.doctor_warnings > 0 {
            spans.push(Span::styled("  doctor:", theme::muted()));
            if stats.doctor_errors > 0 {
                spans.push(Span::styled(format!(" {}E", stats.doctor_errors), theme::danger()));
            }
            if stats.doctor_warnings > 0 {
                spans.push(Span::styled(format!(" {}W", stats.doctor_warnings), theme::warning()));
            }
        } else if state.config_vm.doctor.is_some() {
            spans.push(Span::styled(format!("  {} healthy", glyphs::OK), theme::success()));
        }
        Line::from(spans)
    } else if loading {
        Line::from(Span::styled(
            format!("{} Loading config…", state.spinner_char()),
            theme::muted(),
        ))
    } else {
        Line::from(Span::styled("No mise config files", theme::muted()))
    };
    frame.render_widget(Paragraph::new(line), inner);
}

/// Render the expanded `[4] CONFIG & HEALTH` sidebar showing config hierarchy, trust, and warnings.
pub fn render_sidebar(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Config && state.focus == FocusArea::Sidebar;
    let stats = state.config_vm.stats();
    let loading = state.loading_domains.contains(&Domain::Config);

    let mut title_spans = vec![
        Span::styled(" [4] CONFIG & HEALTH ", theme::panel_title(focused)),
        Span::styled(format!("({}", stats.total_files), theme::muted()),
    ];
    if stats.doctor_errors > 0 {
        title_spans.push(Span::styled(
            format!(" {} {}{}E", glyphs::BULLET, glyphs::FAIL, stats.doctor_errors),
            theme::danger(),
        ));
    }
    if stats.doctor_warnings > 0 {
        title_spans.push(Span::styled(
            format!(" {} {}{}W", glyphs::BULLET, glyphs::WARN, stats.doctor_warnings),
            theme::warning(),
        ));
    }
    if loading {
        title_spans.push(Span::styled(
            format!(" {} {}", glyphs::BULLET, state.spinner_char()),
            theme::accent(),
        ));
    }
    title_spans.push(Span::styled(") ", theme::muted()));

    let right_title = if !state.filters.config.is_empty() {
        Some(Line::from(vec![Span::styled(
            format!(" /{} ", truncate_display(&state.filters.config, 12)),
            theme::highlight_match(),
        )]))
    } else {
        None
    };

    let block = panel_block(Line::from(title_spans), right_title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.register_sidebar_panel(Panel::Config, area, inner);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    if state.config_vm.rows.is_empty() {
        let msg = if loading {
            format!("{} Loading config hierarchy…", state.spinner_char())
        } else if !state.filters.config.is_empty() {
            format!("No config files match '/{}'.", state.filters.config)
        } else {
            "No mise.toml files detected. Press `g` to generate a starter config.".to_string()
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
        .config_vm
        .rows
        .iter()
        .map(|row| {
            let (tg, ts) = trust_glyph(row.trusted);
            let mut badges: Vec<Span<'static>> = Vec::new();
            let mut badges_w = 0usize;

            if row.entry.is_override {
                let s = " [OVR]";
                badges_w += s.len();
                badges.push(Span::styled(s, theme::warning()));
            }
            if row.trusted == Some(false) {
                let s = " [UNTRUSTED]";
                badges_w += s.len();
                badges.push(Span::styled(s, theme::danger().add_modifier(Modifier::BOLD)));
            } else if row.trusted == Some(true) && inner_w > 34 {
                let s = " [TRUSTED]";
                badges_w += s.len();
                badges.push(Span::styled(s, theme::success()));
            }
            if row.explain_warnings > 0 {
                let s = format!(" {}{}", glyphs::WARN, row.explain_warnings);
                badges_w += UnicodeWidthStr::width(s.as_str());
                badges.push(Span::styled(s, theme::warning()));
            }

            let path_budget = inner_w.saturating_sub(2 + badges_w).max(6);
            let short = shorten_path(&row.entry.path, path_budget);
            // If path was shortened, match_indices refer to raw path; highlight cleanly if unchanged.
            let spans_path = if short == row.entry.path {
                highlighted_truncated_spans(&short, &row.match_indices, theme::primary(), path_budget)
            } else {
                vec![Span::styled(short, theme::primary())]
            };

            let mut spans = vec![Span::styled(format!("{tg} "), ts)];
            spans.extend(spans_path);
            spans.extend(badges);
            ListItem::new(Line::from(spans))
        })
        .collect();

    let total_rows = items.len();
    let list = List::new(items).highlight_style(theme::selected(focused));
    frame.render_stateful_widget(list, inner, &mut state.list_states.config);

    render_scrollbar(
        frame,
        area,
        total_rows,
        inner.height as usize,
        state.list_states.config.offset(),
        focused,
    );
}

/// Render the right multi-subview main pane for `[4] CONFIG & HEALTH` (`File & Explain | Doctor | Settings | Daemons & Deps`).
pub fn render_main(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Config && state.focus == FocusArea::Main;
    let labels = ConfigSubView::ALL.map(ConfigSubView::label);
    let active_idx = ConfigSubView::ALL
        .iter()
        .position(|s| *s == state.config_subview)
        .unwrap_or(0);

    let block = main_subtab_block(area, &labels, active_idx, focused, None, layout);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.main_inner = inner;

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    state.detail_scroll.viewport_height = inner.height;

    let lines = match state.config_subview {
        ConfigSubView::File => build_file_lines(state, inner.width as usize),
        ConfigSubView::Doctor => build_doctor_lines(state, inner.width as usize),
        ConfigSubView::Settings => build_settings_lines(state, inner.width as usize),
        ConfigSubView::Services => build_services_lines(state, inner.width as usize),
    };

    let total_lines = lines.len().min(u16::MAX as usize) as u16;
    state.detail_scroll.content_len = total_lines;
    state.detail_scroll.clamp();

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

fn build_file_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(row) = state.selected_config() else {
        return empty_state_lines(
            "No Config File Selected",
            "Select a mise config file from the left hierarchy to inspect its contents and schema diagnostics.",
            Some("Press `g` to open Generate & Scaffold menu, or `D` to run Doctor."),
        );
    };

    let mut lines = Vec::new();
    lines.push(kv_line(
        "FILE",
        shorten_path(&row.entry.path, width.saturating_sub(12)),
        theme::label(),
    ));

    let (tg, ts) = trust_glyph(row.trusted);
    let mut trust_spans = vec![
        Span::styled(format!("{:<10} ", "TRUST"), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{tg} {}", row.trust_status), ts),
    ];
    if row.entry.is_override {
        trust_spans.push(Span::styled("   [OVERRIDE]", theme::warning().add_modifier(Modifier::BOLD)));
    }
    lines.push(Line::from(trust_spans));

    if !row.entry.tools.is_empty() {
        lines.push(kv_line("TOOLS", row.entry.tools.join(", "), theme::success()));
    }

    let diags = state.config_vm.diagnostics_for(&row.entry.path);
    let mut diags_by_line: HashMap<u32, Vec<&ConfigExplainItem>> = HashMap::new();
    for d in &diags {
        diags_by_line.entry(d.line).or_default().push(*d);
    }

    if !diags.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} {} explain diagnostic(s) in this file:", glyphs::WARN, diags.len()),
                theme::warning().add_modifier(Modifier::BOLD),
            ),
        ]));
        for d in &diags {
            let kind = if d.deprecated { "DEPRECATED" } else { "UNKNOWN" };
            let mut msg = format!("  • line {}: [{kind}] `{}`", d.line, d.key);
            if let Some(sug) = &d.suggestion {
                msg.push_str(&format!(" → did you mean `{sug}`?"));
            }
            lines.push(Line::from(Span::styled(msg, theme::warning())));
        }
    }

    lines.push(Line::from(""));
    lines.push(section_divider("CONFIG CONTENT (cached)", width));

    // Strictly zero disk I/O during render: read from `config_vm.file_cache`.
    if let Some(content) = state.config_vm.cached_file_content(&row.entry.path) {
        for (idx, raw) in content.lines().enumerate() {
            let line_no = (idx + 1) as u32;
            let has_diag = diags_by_line.contains_key(&line_no);
            let gutter_style = if has_diag {
                theme::warning().add_modifier(Modifier::BOLD)
            } else {
                theme::subtle()
            };
            let marker = if has_diag { "!" } else { " " };
            let mut spans = vec![Span::styled(format!("{marker}{line_no:>3} │ "), gutter_style)];
            spans.extend(highlight_toml_line(raw).spans);
            lines.push(Line::from(spans));

            if let Some(line_diags) = diags_by_line.get(&line_no) {
                for d in line_diags {
                    let kind = if d.deprecated { "deprecated" } else { "unknown key" };
                    let sug = d
                        .suggestion
                        .as_deref()
                        .map(|s| format!(" — try `{s}`"))
                        .unwrap_or_default();
                    lines.push(Line::from(vec![
                        Span::styled("      ╰─ ", theme::warning()),
                        Span::styled(
                            format!("{} [{kind}] `{}`{sug}", glyphs::WARN, d.key),
                            theme::warning(),
                        ),
                    ]));
                }
            }
        }
    } else {
        lines.push(Line::from(Span::styled(
            "  (file content not cached yet — select file or press `R` to preload, `e` to edit)",
            theme::muted(),
        )));
    }

    lines
}

fn build_doctor_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_divider("MISE DOCTOR HEALTH DASHBOARD", width)];

    if state.loading_domains.contains(&Domain::Doctor) {
        lines.push(Line::from(Span::styled(
            format!("  {} Running `mise doctor --json`…", state.spinner_char()),
            theme::accent(),
        )));
    }

    let Some(summary) = state.config_vm.doctor_summary() else {
        lines.extend(empty_state_lines(
            "Doctor Report Pending",
            "Run `mise doctor` to inspect shell activation, shims, errors, and environment directories.",
            Some("Press `D` to run Doctor now."),
        ));
        return lines;
    };

    // Status summary row
    let health_span = if summary.error_count > 0 {
        Span::styled(
            format!("{} {} ERRORS", glyphs::FAIL, summary.error_count),
            theme::danger().add_modifier(Modifier::BOLD),
        )
    } else if summary.warning_count > 0 {
        Span::styled(
            format!("{} {} WARNINGS", glyphs::WARN, summary.warning_count),
            theme::warning().add_modifier(Modifier::BOLD),
        )
    } else {
        Span::styled(
            format!("{} HEALTHY", glyphs::OK),
            theme::success().add_modifier(Modifier::BOLD),
        )
    };

    lines.push(Line::from(vec![
        Span::styled(format!("{:<10} ", "HEALTH"), theme::muted().add_modifier(Modifier::BOLD)),
        health_span,
        Span::styled("   MISE: ", theme::muted()),
        Span::styled(
            if summary.version.is_empty() {
                "unknown".to_string()
            } else {
                summary.version.clone()
            },
            theme::primary(),
        ),
    ]));

    if let Some(shell) = &summary.shell_display {
        lines.push(kv_line("SHELL", shell.clone(), theme::primary()));
    }

    lines.push(Line::from(vec![
        Span::styled(format!("{:<10} ", "ACTIVATED"), theme::muted().add_modifier(Modifier::BOLD)),
        if summary.activated {
            Span::styled(format!("{} yes", glyphs::OK), theme::success())
        } else {
            Span::styled(format!("{} no", glyphs::WARN), theme::warning())
        },
        Span::styled("   SHIMS ON PATH: ", theme::muted()),
        if summary.shims_on_path {
            Span::styled(format!("{} yes", glyphs::OK), theme::success())
        } else {
            Span::styled(format!("{} no", glyphs::IDLE), theme::muted())
        },
        if summary.self_update_available {
            Span::styled("   ↑ update available", theme::warning())
        } else {
            Span::raw("")
        },
    ]));

    if !summary.errors.is_empty() {
        lines.push(Line::from(""));
        lines.push(section_divider(&format!("ERRORS ({})", summary.errors.len()), width));
        for err in &summary.errors {
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", glyphs::FAIL), theme::danger().add_modifier(Modifier::BOLD)),
                Span::styled(err.clone(), theme::danger()),
            ]));
        }
    }

    if !summary.warnings.is_empty() {
        lines.push(Line::from(""));
        lines.push(section_divider(&format!("WARNINGS ({})", summary.warnings.len()), width));
        for warn in &summary.warnings {
            lines.push(Line::from(vec![
                Span::styled(format!("  {} ", glyphs::WARN), theme::warning().add_modifier(Modifier::BOLD)),
                Span::styled(warn.clone(), theme::warning()),
            ]));
        }
    }

    if !summary.dirs.is_empty() {
        lines.push(Line::from(""));
        lines.push(section_divider("DIRECTORIES", width));
        for (k, v) in &summary.dirs {
            lines.push(kv_line(
                &k.to_ascii_uppercase(),
                shorten_path(v, width.saturating_sub(14)),
                theme::muted(),
            ));
        }
    }

    if !summary.ignored_config_files.is_empty() {
        lines.push(Line::from(""));
        lines.push(section_divider(
            &format!("IGNORED CONFIG FILES ({})", summary.ignored_config_files.len()),
            width,
        ));
        for p in &summary.ignored_config_files {
            lines.push(Line::from(Span::styled(
                format!("  • {}", shorten_path(p, width.saturating_sub(6))),
                theme::muted(),
            )));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Quick Fixes: ", theme::label()),
        Span::styled("D: refresh doctor   x: menu (mise reshim / doctor project / fmt)", theme::muted()),
    ]));
    lines
}

fn build_settings_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let matches = state.config_vm.search_settings(&state.filters.config);
    let mut lines = vec![section_divider(
        &format!("MISE SETTINGS ({}/{})", matches.len(), state.config_vm.settings.len()),
        width,
    )];

    if state.loading_domains.contains(&Domain::Settings) && state.config_vm.settings.is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {} Loading `mise settings ls --json-extended`…", state.spinner_char()),
            theme::accent(),
        )));
        return lines;
    }

    if matches.is_empty() {
        lines.extend(empty_state_lines(
            "No Matching Settings",
            if state.filters.config.is_empty() {
                "No settings loaded yet."
            } else {
                "No mise settings matched the current `/` filter."
            },
            Some("Press `/` to fuzzy-search settings by key, value, source, or description."),
        ));
        return lines;
    }

    lines.push(Line::from(vec![
        Span::styled(
            format!("  {} ", pad_display_width("SETTING", 28)),
            theme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ", pad_display_width("VALUE", 16)),
            theme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{} ", pad_display_width("TYPE", 8)),
            theme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled("SOURCE / DESC", theme::muted().add_modifier(Modifier::BOLD)),
    ]));

    let desc_budget = width.saturating_sub(58).max(12);
    for m in matches.iter().take(200) {
        let s = &m.entry;
        let val_str = s
            .value
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| s.value.to_string());
        let val_style = match val_str.as_str() {
            "true" => theme::success(),
            "false" => theme::muted(),
            _ => theme::warning(),
        };
        let extra = s
            .source
            .as_deref()
            .filter(|src| !src.is_empty())
            .map(|src| format!("[{}] {}", shorten_path(src, 20), s.description))
            .unwrap_or_else(|| s.description.clone());

        let mut spans = vec![Span::raw("  ")];
        spans.extend(highlighted_truncated_spans(
            &s.key,
            &m.match_indices,
            theme::accent(),
            28,
        ));
        let key_w = UnicodeWidthStr::width(truncate_display(&s.key, 28).as_str());
        if key_w < 28 {
            spans.push(Span::raw(" ".repeat(28 - key_w)));
        }
        spans.push(Span::raw(" "));
        spans.push(Span::styled(
            format!("{} ", pad_display_width(&val_str, 16)),
            val_style,
        ));
        spans.push(Span::styled(
            format!("{} ", pad_display_width(&s.kind, 8)),
            theme::subtle(),
        ));
        spans.push(Span::styled(
            truncate_display(&extra, desc_budget),
            theme::muted(),
        ));
        lines.push(Line::from(spans));
    }

    lines
}

fn build_services_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let mut lines = Vec::new();
    let has_daemons = !state.config_vm.daemons.is_empty();
    let has_deps = !state.config_vm.deps.is_empty();

    if !has_daemons && !has_deps {
        lines.push(section_divider("DAEMONS & PROJECT DEPS", width));
        lines.extend(empty_state_lines(
            "No `[daemons]` or `[deps]` Configured",
            "This workspace does not declare mise background daemons or declarative dependency providers.",
            Some("Add `[daemons.<name>]` or `[deps]` in `mise.toml` to manage local services & sync here."),
        ));
        return lines;
    }

    lines.push(section_divider(
        &format!("DAEMONS ({})", state.config_vm.daemons.len()),
        width,
    ));
    if !has_daemons {
        lines.push(Line::from(Span::styled(
            "  No daemons configured.",
            theme::muted(),
        )));
    } else {
        for (idx, daemon) in state.config_vm.daemons.iter().enumerate() {
            let is_selected = idx == state.selected_daemon_idx;
            let running = daemon.status.eq_ignore_ascii_case("running")
                || daemon.status.eq_ignore_ascii_case("active")
                || daemon.status.eq_ignore_ascii_case("started");
            let (glyph, gstyle) = if running {
                (glyphs::RUNNING, theme::success())
            } else {
                (glyphs::IDLE, theme::muted())
            };
            let pid_str = daemon
                .pid
                .map(|p| format!("  pid:{p}"))
                .unwrap_or_default();
            lines.push(Line::from(vec![
                Span::styled(
                    if is_selected { "▸ " } else { "  " },
                    if is_selected { theme::label() } else { theme::muted() },
                ),
                Span::styled(format!("{glyph} "), gstyle),
                Span::styled(
                    pad_display_width(&daemon.name, 18),
                    if is_selected {
                        theme::label()
                    } else {
                        theme::section_header()
                    },
                ),
                Span::styled(format!(" {:<10}", daemon.status), gstyle),
                Span::styled(pid_str, theme::muted()),
                if let Some(url) = &daemon.url {
                    Span::styled(format!("  {url}"), theme::accent())
                } else {
                    Span::raw("")
                },
            ]));
            if let Some(cmd) = &daemon.command {
                lines.push(Line::from(Span::styled(
                    format!("      $ {}", truncate_display(cmd, width.saturating_sub(10))),
                    theme::muted(),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(section_divider(
        &format!("PROJECT DEPS ({})", state.config_vm.deps.len()),
        width,
    ));
    if !has_deps {
        lines.push(Line::from(Span::styled(
            "  No deps providers configured.",
            theme::muted(),
        )));
    } else {
        for dep in &state.config_vm.deps {
            let fresh = dep.status.eq_ignore_ascii_case("fresh")
                || dep.status.eq_ignore_ascii_case("ok")
                || dep.status.eq_ignore_ascii_case("synced");
            let st_style = if fresh {
                theme::success()
            } else {
                theme::warning()
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!("  {} ", if fresh { glyphs::OK } else { glyphs::WARN }),
                    st_style,
                ),
                Span::styled(pad_display_width(&dep.name, 16), theme::primary()),
                Span::styled(format!(" {:<12} ", dep.status), st_style),
                Span::styled(dep.detail.clone(), theme::muted()),
            ]));
        }
    }

    lines
}

fn trust_glyph(trusted: Option<bool>) -> (&'static str, ratatui::style::Style) {
    match trusted {
        Some(true) => (glyphs::OK, theme::success()),
        Some(false) => (glyphs::WARN, theme::danger()),
        None => ("?", theme::muted()),
    }
}
