use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::action::{Domain, FocusArea, Panel, ToolScope, ToolSubView};
use crate::app::state::AppState;
use crate::model::tools::ToolRow;
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};
use crate::ui::widgets::{
    empty_state_lines, highlighted_truncated_spans, kv_line, main_subtab_block, pad_display_width,
    panel_block, render_scrollbar, section_divider, shorten_path, truncate_display,
};

const ALL_SCOPES: [ToolScope; 5] = [
    ToolScope::All,
    ToolScope::Active,
    ToolScope::Project,
    ToolScope::Missing,
    ToolScope::Prunable,
];

/// Render the collapsed accordion summary card (1-line or 3-line) for `[2] TOOLS`.
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
    layout.register_sidebar_panel(Panel::Tools, area, Rect::default());

    let stats = state.tools_vm.stats(&state.workdir);
    let loading = state.loading_domains.contains(&Domain::Tools);

    let mut badge_spans = vec![Span::styled(format!("({}", stats.total), theme::muted())];
    if stats.outdated > 0 {
        badge_spans.push(Span::styled(
            format!(" {} {}{}", glyphs::BULLET, glyphs::UPGRADE, stats.outdated),
            theme::warning(),
        ));
    }
    if stats.vulnerable > 0 {
        badge_spans.push(Span::styled(
            format!(" {} {} {} CVE", glyphs::BULLET, glyphs::WARN, stats.vulnerable),
            theme::danger(),
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
            Span::styled("[2] TOOLS ", theme::panel_title(false)),
        ];
        spans.extend(badge_spans);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let mut title_spans = vec![Span::styled(" [2] TOOLS ", theme::panel_title(false))];
    title_spans.extend(badge_spans);
    let block = panel_block(Line::from(title_spans), None, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let line = if loading && state.tools_vm.rows.is_empty() {
        Line::from(Span::styled(
            format!("{} Loading tools…", state.spinner_char()),
            theme::muted(),
        ))
    } else if state.tools_vm.rows.is_empty() {
        Line::from(Span::styled("No tools in scope", theme::muted()))
    } else {
        let mut spans = Vec::new();
        let mut used = 0usize;
        let max_w = inner.width as usize;
        for row in state.tools_vm.rows.iter().take(4) {
            let (g, gs) = tool_status_glyph(row);
            let ver = if let Some(out) = &row.outdated {
                format!("{}→{}", row.entry.version, out.latest)
            } else {
                row.entry.version.clone()
            };
            let token = format!("{g} {} {ver}  ", row.name);
            let tw = UnicodeWidthStr::width(token.as_str());
            if used + tw > max_w && !spans.is_empty() {
                break;
            }
            spans.push(Span::styled(format!("{g} "), gs));
            spans.push(Span::styled(format!("{} ", row.name), theme::primary()));
            spans.push(Span::styled(
                format!("{ver}  "),
                if row.outdated.is_some() {
                    theme::warning()
                } else {
                    theme::muted()
                },
            ));
            used += tw;
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), inner);
}

/// Render the expanded `[2] TOOLS` sidebar with scope pills, upgrade arrows, and CVE badges.
pub fn render_sidebar(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Tools && state.focus == FocusArea::Sidebar;
    let stats = state.tools_vm.stats(&state.workdir);
    let loading = state.loading_domains.contains(&Domain::Tools);

    let mut title_spans = vec![
        Span::styled(" [2] TOOLS ", theme::panel_title(focused)),
        Span::styled(format!("({}", stats.total), theme::muted()),
    ];
    if stats.outdated > 0 {
        title_spans.push(Span::styled(
            format!(" {} {}{}", glyphs::BULLET, glyphs::UPGRADE, stats.outdated),
            theme::warning(),
        ));
    }
    if stats.vulnerable > 0 {
        title_spans.push(Span::styled(
            format!(" {} {}{}", glyphs::BULLET, glyphs::WARN, stats.vulnerable),
            theme::danger(),
        ));
    }
    if loading {
        title_spans.push(Span::styled(
            format!(" {} {}", glyphs::BULLET, state.spinner_char()),
            theme::accent(),
        ));
    }
    title_spans.push(Span::styled(") ", theme::muted()));

    let right_title = if !state.filters.tools.is_empty() {
        Some(Line::from(vec![
            Span::styled(
                format!(" /{} ", truncate_display(&state.filters.tools, 12)),
                theme::highlight_match(),
            ),
        ]))
    } else {
        None
    };

    let block = panel_block(Line::from(title_spans), right_title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        layout.register_sidebar_panel(Panel::Tools, area, Rect::default());
        return;
    }

    let (scope_area, list_area) = if inner.height >= 3 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, inner)
    };

    layout.register_sidebar_panel(Panel::Tools, area, list_area);

    if let Some(sa) = scope_area {
        let mut pill_spans = Vec::new();
        for scope in ALL_SCOPES {
            let active = scope == state.tool_scope;
            let count = match scope {
                ToolScope::All => stats.total,
                ToolScope::Active => stats.active,
                ToolScope::Project => stats.project,
                ToolScope::Missing => stats.missing,
                ToolScope::Prunable => stats.prunable,
            };
            let label = if active {
                format!("[{}:{count}]", scope.label())
            } else {
                format!(" {} ", scope.label())
            };
            pill_spans.push(Span::styled(label, theme::filter_pill(active)));
        }
        frame.render_widget(Paragraph::new(Line::from(pill_spans)), sa);
    }

    if state.tools_vm.rows.is_empty() {
        let msg = if loading {
            format!("{} Loading installed tools…", state.spinner_char())
        } else if !state.filters.tools.is_empty() {
            format!("No tools match '/{}' in scope [{}].", state.filters.tools, state.tool_scope.label())
        } else {
            format!("No tools in scope [{}] (f: cycle scope, a: add from registry).", state.tool_scope.label())
        };
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(msg, theme::muted())))
                .wrap(Wrap { trim: true }),
            list_area,
        );
        return;
    }

    let inner_w = list_area.width as usize;
    let items: Vec<ListItem<'static>> = state
        .tools_vm
        .rows
        .iter()
        .map(|row| {
            let (glyph, gstyle) = tool_status_glyph(row);
            let mut right_spans: Vec<Span<'static>> = Vec::new();
            let mut right_w = 0usize;

            if let Some(out) = &row.outdated {
                let ver_text = format!(" {} → {}", row.entry.version, out.latest);
                right_w += UnicodeWidthStr::width(ver_text.as_str());
                right_spans.push(Span::styled(format!(" {}", row.entry.version), theme::muted()));
                right_spans.push(Span::styled(" → ", theme::warning()));
                right_spans.push(Span::styled(out.latest.clone(), theme::warning().add_modifier(Modifier::BOLD)));
            } else if !row.entry.version.is_empty() {
                let ver_text = format!(" {}", row.entry.version);
                right_w += UnicodeWidthStr::width(ver_text.as_str());
                right_spans.push(Span::styled(
                    ver_text,
                    if row.entry.installed {
                        theme::muted()
                    } else {
                        theme::danger()
                    },
                ));
            }

            if !row.vulnerabilities.is_empty() {
                let badge = format!(" {}{}", glyphs::WARN, row.vulnerabilities.len());
                right_w += UnicodeWidthStr::width(badge.as_str());
                right_spans.push(Span::styled(badge, theme::danger().add_modifier(Modifier::BOLD)));
            }
            if row.is_project_local && inner_w > 30 {
                let badge = " proj";
                right_w += badge.len();
                right_spans.push(Span::styled(badge, theme::subtle()));
            }

            let name_budget = inner_w.saturating_sub(2 + right_w).max(5);
            let name_style = if row.entry.active {
                theme::primary()
            } else {
                theme::muted()
            };

            let mut spans = vec![Span::styled(format!("{glyph} "), gstyle)];
            spans.extend(highlighted_truncated_spans(
                &row.name,
                &row.match_indices,
                name_style,
                name_budget,
            ));
            spans.extend(right_spans);
            ListItem::new(Line::from(spans))
        })
        .collect();

    let total_rows = items.len();
    let list = List::new(items).highlight_style(theme::selected(focused));
    frame.render_stateful_widget(list, list_area, &mut state.list_states.tools);

    render_scrollbar(
        frame,
        area,
        total_rows,
        list_area.height as usize,
        state.list_states.tools.offset(),
        focused,
    );
}

/// Render the right multi-subview main pane for `[2] TOOLS` (`Details | Outdated | Audit | Lock`).
pub fn render_main(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Tools && state.focus == FocusArea::Main;
    let labels = ToolSubView::ALL.map(ToolSubView::label);
    let active_idx = ToolSubView::ALL
        .iter()
        .position(|s| *s == state.tool_subview)
        .unwrap_or(0);

    let block = main_subtab_block(area, &labels, active_idx, focused, None, layout);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.main_inner = inner;

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    state.detail_scroll.viewport_height = inner.height;

    let lines = match state.tool_subview {
        ToolSubView::Details => build_details_lines(state, inner.width as usize),
        ToolSubView::Outdated => build_outdated_lines(state, inner.width as usize),
        ToolSubView::Audit => build_audit_lines(state, inner.width as usize),
        ToolSubView::Lock => build_lock_lines(state, inner.width as usize),
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

fn build_details_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(row) = state.selected_tool() else {
        return empty_state_lines(
            "No Tool Selected",
            "Select a tool on the left to inspect its installation, backend, and security status.",
            Some("Press `a` to search & install from the mise registry (1,079+ tools)."),
        );
    };

    let mut lines = Vec::new();
    lines.push(kv_line("TOOL", row.name.clone(), theme::label()));

    let version_display = if let Some(out) = &row.outdated {
        format!("{}  ({} latest: {})", row.entry.version, glyphs::UPGRADE, out.latest)
    } else {
        row.entry.version.clone()
    };
    lines.push(kv_line(
        "VERSION",
        version_display,
        if row.outdated.is_some() {
            theme::warning()
        } else {
            theme::primary()
        },
    ));

    if let Some(req) = &row.entry.requested_version {
        lines.push(kv_line("REQUESTED", req.clone(), theme::accent()));
    }

    let mut status_bits = Vec::new();
    if row.entry.active {
        status_bits.push(format!("{} active", glyphs::OK));
    } else {
        status_bits.push(format!("{} inactive", glyphs::INACTIVE));
    }
    if row.entry.installed {
        status_bits.push("installed".to_string());
    } else {
        status_bits.push(format!("{} missing", glyphs::FAIL));
    }
    if row.entry.scheduled_for_pruning {
        status_bits.push(format!("{} prunable", glyphs::PRUNE));
    }
    lines.push(kv_line(
        "STATUS",
        status_bits.join(" • "),
        if !row.entry.installed {
            theme::danger()
        } else if row.entry.active {
            theme::success()
        } else {
            theme::muted()
        },
    ));

    lines.push(kv_line(
        "SCOPE",
        if row.is_project_local {
            "project-local"
        } else {
            "global / inherited"
        },
        theme::primary(),
    ));

    if let Some(src) = &row.entry.source {
        let src_str = match (&src.kind, &src.path) {
            (Some(k), Some(p)) => format!("{k} — {}", shorten_path(p, width.saturating_sub(24))),
            (None, Some(p)) => shorten_path(p, width.saturating_sub(12)),
            (Some(k), None) => k.clone(),
            (None, None) => "—".to_string(),
        };
        lines.push(kv_line("SOURCE", src_str, theme::muted()));
    }

    if let Some(install_path) = &row.entry.install_path {
        lines.push(kv_line(
            "INSTALL",
            shorten_path(install_path, width.saturating_sub(12)),
            theme::muted(),
        ));
    }
    if let Some(prune) = &row.entry.prune_after {
        lines.push(kv_line("PRUNE", prune.clone(), theme::warning()));
    }

    if !row.vulnerabilities.is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(vec![
            Span::styled(
                format!("{} {} KNOWN VULNERABILITY(IES) ", glyphs::WARN, row.vulnerabilities.len()),
                theme::danger().add_modifier(Modifier::BOLD),
            ),
            Span::styled("(switch to Audit tab or run `mise upgrade --vulnerable`)", theme::muted()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(section_divider("BACKEND & METADATA", width));

    if let Some(detail) = state.tools_vm.detail_for(&row.name) {
        if let Some(backend) = &detail.backend {
            lines.push(kv_line("BACKEND", backend.clone(), theme::purple()));
        }
        if let Some(desc) = &detail.description {
            lines.push(kv_line("DESC", desc.clone(), theme::primary()));
        }
        if let Some(url) = &detail.url {
            lines.push(kv_line("URL", url.clone(), theme::accent()));
        }
        if !detail.installed_versions.is_empty() {
            lines.push(kv_line(
                "INSTALLED",
                detail.installed_versions.join(", "),
                theme::muted(),
            ));
        }
        if !detail.active_versions.is_empty() {
            lines.push(kv_line(
                "ACTIVE",
                detail.active_versions.join(", "),
                theme::success(),
            ));
        }
        if let Some(opts) = &detail.tool_options
            && !opts.is_null()
        {
            lines.push(kv_line("OPTIONS", opts.to_string(), theme::muted()));
        }
    } else if state.loading_domains.contains(&Domain::ToolDetail) {
        lines.push(Line::from(Span::styled(
            format!("  {} Fetching `mise tool {} --json`…", state.spinner_char(), row.name),
            theme::accent(),
        )));
    } else {
        lines.push(Line::from(Span::styled(
            "  Tool metadata not loaded yet.",
            theme::muted(),
        )));
    }

    lines
}

fn build_outdated_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_divider(
        &format!("OUTDATED TOOLS ({})", state.tools_vm.outdated.len()),
        width,
    )];

    if state.loading_domains.contains(&Domain::Outdated) {
        lines.push(Line::from(Span::styled(
            format!("  {} Checking `mise outdated --json`…", state.spinner_char()),
            theme::accent(),
        )));
    }

    if state.tools_vm.outdated.is_empty() {
        lines.extend(empty_state_lines(
            &format!("{} All Checked Tools Are Up To Date", glyphs::OK),
            "No outdated tool versions were reported by mise.",
            Some("Press `O` to refresh outdated check, or `U` to upgrade all."),
        ));
        return lines;
    }

    lines.push(Line::from(vec![
        Span::styled(format!("  {} ", pad_display_width("TOOL", 16)), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} ", pad_display_width("CURRENT", 12)), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} ", pad_display_width("REQUESTED", 12)), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled(format!("{} ", pad_display_width("LATEST", 12)), theme::muted().add_modifier(Modifier::BOLD)),
        Span::styled("RELEASE", theme::muted().add_modifier(Modifier::BOLD)),
    ]));

    let selected_name = state.selected_tool().map(|t| t.name.as_str());
    for (name, entry) in &state.tools_vm.outdated {
        let is_sel = selected_name == Some(name.as_str());
        let cur = entry.current.as_deref().unwrap_or("—");
        let req = entry.requested.as_deref().unwrap_or("—");
        let rel = entry
            .release_url
            .as_deref()
            .unwrap_or(entry.bump.as_deref().unwrap_or("—"));

        lines.push(Line::from(vec![
            Span::styled(
                if is_sel { "▸ " } else { "  " },
                if is_sel { theme::label() } else { theme::muted() },
            ),
            Span::styled(
                format!("{} ", pad_display_width(name, 16)),
                if is_sel { theme::label() } else { theme::primary() },
            ),
            Span::styled(format!("{} ", pad_display_width(cur, 12)), theme::muted()),
            Span::styled(format!("{} ", pad_display_width(req, 12)), theme::muted()),
            Span::styled(
                format!("{} ", pad_display_width(&entry.latest, 12)),
                theme::warning().add_modifier(Modifier::BOLD),
            ),
            Span::styled(truncate_display(rel, width.saturating_sub(58)), theme::accent()),
        ]));
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Actions: ", theme::label()),
        Span::styled("u: upgrade selected   U: upgrade all   O: refresh   o: open release URL", theme::muted()),
    ]));
    lines
}

fn build_audit_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_divider("SECURITY AUDIT (OSV.dev)", width)];

    if state.loading_domains.contains(&Domain::Audit) {
        lines.push(Line::from(Span::styled(
            format!("  {} Running `mise audit --json`…", state.spinner_char()),
            theme::accent(),
        )));
    }

    let Some(report) = &state.tools_vm.audit else {
        lines.extend(empty_state_lines(
            "Security Audit Not Run Yet",
            "Run `mise audit` to scan active tool versions against OSV vulnerability advisories.",
            Some("Press `A` in Tools panel to run security audit."),
        ));
        return lines;
    };

    if report.vulnerabilities.is_empty() {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", glyphs::OK), theme::success().add_modifier(Modifier::BOLD)),
            Span::styled("No known vulnerabilities detected.", theme::success()),
        ]));
    } else {
        lines.push(Line::from(vec![
            Span::styled(
                format!("  {} {} vulnerabilities found", glyphs::WARN, report.vulnerabilities.len()),
                theme::danger().add_modifier(Modifier::BOLD),
            ),
            Span::styled("  (use x menu → upgrade vulnerable)", theme::muted()),
        ]));
        lines.push(Line::from(""));

        for vuln in &report.vulnerabilities {
            let sev = vuln.severity.as_deref().unwrap_or("UNKNOWN").to_ascii_uppercase();
            let sev_style = match sev.as_str() {
                "CRITICAL" | "HIGH" => theme::danger().add_modifier(Modifier::BOLD),
                "MEDIUM" | "MODERATE" => theme::warning().add_modifier(Modifier::BOLD),
                _ => theme::accent(),
            };
            let alias_str = if vuln.aliases.is_empty() {
                String::new()
            } else {
                format!(" ({})", vuln.aliases.join(", "))
            };
            lines.push(Line::from(vec![
                Span::styled(format!("  [{sev}] "), sev_style),
                Span::styled(format!("{}@{}", vuln.tool, vuln.version), theme::section_header()),
                Span::styled("  ", theme::muted()),
                Span::styled(vuln.id.clone(), theme::accent()),
                Span::styled(alias_str, theme::muted()),
            ]));
            if !vuln.summary.is_empty() {
                lines.push(Line::from(Span::styled(
                    format!("    {}", vuln.summary),
                    theme::primary(),
                )));
            }
            if !vuln.fixed.is_empty() {
                lines.push(Line::from(vec![
                    Span::styled("    Fixed in: ", theme::muted()),
                    Span::styled(vuln.fixed.join(", "), theme::success()),
                ]));
            }
            if let Some(url) = &vuln.url {
                lines.push(Line::from(Span::styled(format!("    {url}"), theme::muted())));
            }
            lines.push(Line::from(""));
        }
    }

    if !report.unchecked.is_empty() {
        lines.push(section_divider(
            &format!("UNCHECKED TOOLS ({})", report.unchecked.len()),
            width,
        ));
        let names = report
            .unchecked
            .iter()
            .map(|u| format!("{}@{}", u.tool, u.version))
            .collect::<Vec<_>>()
            .join(", ");
        lines.push(Line::from(Span::styled(format!("  {names}"), theme::muted())));
    }

    lines
}

fn build_lock_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let mut lines = vec![section_divider("MISE.LOCK DRY-RUN PREVIEW", width)];

    if state.loading_domains.contains(&Domain::LockDryRun) {
        lines.push(Line::from(Span::styled(
            format!("  {} Computing `mise lock --dry-run`…", state.spinner_char()),
            theme::accent(),
        )));
        return lines;
    }

    let Some(preview) = state.tools_vm.lock_preview.as_deref() else {
        lines.extend(empty_state_lines(
            "Lockfile Preview Ready",
            "Preview checksum & version pin changes before writing `mise.lock`.",
            Some("Press `L` to refresh `mise lock --dry-run`, or `x` → Apply lockfile (`mise lock`)."),
        ));
        return lines;
    };

    if preview.trim().is_empty() {
        lines.push(Line::from(Span::styled(
            format!("  {} Lockfile is already up to date (no changes).", glyphs::OK),
            theme::success(),
        )));
    } else {
        for raw in preview.lines() {
            let style = if raw.starts_with('+') && !raw.starts_with("+++") {
                theme::success()
            } else if raw.starts_with('-') && !raw.starts_with("---") {
                theme::danger()
            } else if raw.starts_with("@@") || raw.starts_with('[') {
                theme::accent()
            } else {
                theme::primary()
            };
            lines.push(Line::from(Span::styled(format!("  {raw}"), style)));
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Action: ", theme::label()),
        Span::styled(
            "Press `L` to refresh dry-run, or `x` → Apply lockfile (`mise lock`) to update mise.lock (with confirmation).",
            theme::muted(),
        ),
    ]));
    lines
}

fn tool_status_glyph(row: &ToolRow) -> (&'static str, ratatui::style::Style) {
    if !row.vulnerabilities.is_empty() {
        (glyphs::WARN, theme::danger())
    } else if !row.entry.installed {
        (glyphs::FAIL, theme::danger())
    } else if row.outdated.is_some() {
        (glyphs::UPGRADE, theme::warning())
    } else if row.entry.scheduled_for_pruning {
        (glyphs::PRUNE, theme::warning())
    } else if row.entry.active {
        (glyphs::OK, theme::success())
    } else {
        (glyphs::INACTIVE, theme::muted())
    }
}
