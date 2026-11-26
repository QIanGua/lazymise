use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{List, ListItem, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::action::{Domain, EnvFilter, EnvSubView, FocusArea, Panel};
use crate::app::state::AppState;
use crate::model::env::{EnvRow, EnvSourceKind};
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};
use crate::ui::widgets::{
    empty_state_lines, highlighted_truncated_spans, kv_line, main_subtab_block, pad_display_width,
    panel_block, render_scrollbar, section_divider, shorten_path, truncate_display,
};

const ALL_ENV_FILTERS: [EnvFilter; 6] = [
    EnvFilter::All,
    EnvFilter::Project,
    EnvFilter::Local,
    EnvFilter::Global,
    EnvFilter::ToolInjected,
    EnvFilter::Redacted,
];

/// Render the collapsed accordion summary card (1-line or 3-line) for `[3] ENV`.
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
    layout.register_sidebar_panel(Panel::Env, area, Rect::default());

    let stats = state.env_vm.stats(&state.workdir);
    let loading = state.loading_domains.contains(&Domain::Env);

    let mut badge_spans = vec![Span::styled(format!("({}", stats.total), theme::muted())];
    if stats.redacted > 0 {
        badge_spans.push(Span::styled(
            format!(" {} {}{}", glyphs::BULLET, glyphs::LOCK, stats.redacted),
            theme::warning(),
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
            Span::styled("[3] ENV ", theme::panel_title(false)),
        ];
        spans.extend(badge_spans);
        frame.render_widget(Paragraph::new(Line::from(spans)), area);
        return;
    }

    let mut title_spans = vec![Span::styled(" [3] ENV ", theme::panel_title(false))];
    title_spans.extend(badge_spans);
    let block = panel_block(Line::from(title_spans), None, false);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let line = if loading && state.env_vm.rows.is_empty() {
        Line::from(Span::styled(
            format!("{} Loading environment…", state.spinner_char()),
            theme::muted(),
        ))
    } else if state.env_vm.rows.is_empty() {
        Line::from(Span::styled("No environment variables", theme::muted()))
    } else {
        let mut spans = Vec::new();
        let mut used = 0usize;
        let max_w = inner.width as usize;
        for (i, row) in state.env_vm.rows.iter().take(5).enumerate() {
            if i > 0 {
                spans.push(Span::styled(", ", theme::subtle()));
                used += 2;
            }
            let prefix = if row.sensitive { glyphs::LOCK } else { "" };
            let token = format!("{prefix}{}", row.key);
            let tw = UnicodeWidthStr::width(token.as_str());
            if used + tw > max_w && !spans.is_empty() {
                spans.push(Span::styled("…", theme::muted()));
                break;
            }
            if row.sensitive {
                spans.push(Span::styled(glyphs::LOCK, theme::warning()));
            }
            spans.push(Span::styled(row.key.clone(), theme::primary()));
            used += tw;
        }
        Line::from(spans)
    };
    frame.render_widget(Paragraph::new(line), inner);
}

/// Render the expanded `[3] ENV` sidebar with source filter pills, source badges, and redaction locks.
pub fn render_sidebar(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Env && state.focus == FocusArea::Sidebar;
    let stats = state.env_vm.stats(&state.workdir);
    let loading = state.loading_domains.contains(&Domain::Env);

    let mut title_spans = vec![
        Span::styled(" [3] ENV ", theme::panel_title(focused)),
        Span::styled(format!("({}", stats.total), theme::muted()),
    ];
    if stats.redacted > 0 {
        title_spans.push(Span::styled(
            format!(" {} {}{}", glyphs::BULLET, glyphs::LOCK, stats.redacted),
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

    let right_title = if !state.filters.env.is_empty() {
        Some(Line::from(vec![Span::styled(
            format!(" /{} ", truncate_display(&state.filters.env, 12)),
            theme::highlight_match(),
        )]))
    } else {
        None
    };

    let block = panel_block(Line::from(title_spans), right_title, focused);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 || inner.width == 0 {
        layout.register_sidebar_panel(Panel::Env, area, Rect::default());
        return;
    }

    let (filter_area, list_area) = if inner.height >= 3 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Min(1)])
            .split(inner);
        (Some(chunks[0]), chunks[1])
    } else {
        (None, inner)
    };

    layout.register_sidebar_panel(Panel::Env, area, list_area);

    if let Some(fa) = filter_area {
        let mut pill_spans = Vec::new();
        for f in ALL_ENV_FILTERS {
            let active = f == state.env_filter;
            let short = match f {
                EnvFilter::All => "all",
                EnvFilter::Project => "proj",
                EnvFilter::Local => "loc",
                EnvFilter::Global => "glob",
                EnvFilter::ToolInjected => "tool",
                EnvFilter::Redacted => "🔒",
            };
            let label = if active {
                format!("[{short}]")
            } else {
                format!(" {short} ")
            };
            pill_spans.push(Span::styled(label, theme::filter_pill(active)));
        }
        frame.render_widget(Paragraph::new(Line::from(pill_spans)), fa);
    }

    if state.env_vm.rows.is_empty() {
        let msg = if loading {
            format!("{} Loading environment variables…", state.spinner_char())
        } else if !state.filters.env.is_empty() {
            format!("No env vars match '/{}'.", state.filters.env)
        } else {
            format!(
                "No variables in filter [{}] (f: cycle filter, a: mise set).",
                state.env_filter.label()
            )
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
        .env_vm
        .rows
        .iter()
        .map(|row| {
            let revealed = state.is_env_revealed(&row.key);
            let (icon, istyle) = env_row_icon(row, revealed);
            let (tag, tag_style) = source_tag(row.source_kind);
            let tag_str = format!(" [{tag}]");
            let tag_w = UnicodeWidthStr::width(tag_str.as_str());
            let icon_w = UnicodeWidthStr::width(icon) + 1;
            let key_budget = inner_w.saturating_sub(icon_w + tag_w).max(4);

            let mut spans = vec![Span::styled(format!("{icon} "), istyle)];
            spans.extend(highlighted_truncated_spans(
                &row.key,
                &row.match_indices,
                theme::primary(),
                key_budget,
            ));
            spans.push(Span::styled(tag_str, tag_style));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let total_rows = items.len();
    let list = List::new(items).highlight_style(theme::selected(focused));
    frame.render_stateful_widget(list, list_area, &mut state.list_states.env);

    render_scrollbar(
        frame,
        area,
        total_rows,
        list_area.height as usize,
        state.list_states.env.offset(),
        focused,
    );
}

/// Render the right multi-subview main pane for `[3] ENV` (`Value & Source | PATH | Vars`).
pub fn render_main(
    frame: &mut Frame,
    state: &mut AppState,
    area: Rect,
    layout: &mut LayoutMap,
) {
    if area.height == 0 || area.width == 0 {
        return;
    }

    let focused = state.active_panel == Panel::Env && state.focus == FocusArea::Main;
    let labels = EnvSubView::ALL.map(EnvSubView::label);
    let active_idx = EnvSubView::ALL
        .iter()
        .position(|s| *s == state.env_subview)
        .unwrap_or(0);

    let block = main_subtab_block(area, &labels, active_idx, focused, None, layout);
    let inner = block.inner(area);
    frame.render_widget(block, area);
    layout.main_inner = inner;

    if inner.height == 0 || inner.width == 0 {
        return;
    }

    state.detail_scroll.viewport_height = inner.height;

    let lines = match state.env_subview {
        EnvSubView::Detail => build_detail_lines(state, inner.width as usize),
        EnvSubView::Paths => build_paths_lines(state, inner.width as usize),
        EnvSubView::Vars => build_vars_lines(state, inner.width as usize),
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

fn build_detail_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let Some(row) = state.selected_env() else {
        return empty_state_lines(
            "No Environment Variable Selected",
            "Select a variable from the left sidebar to inspect its source and value.",
            Some("Press `a` to add a variable (`mise set`), `S` to reveal secrets, or `y` to copy."),
        );
    };

    let reveal_remaining_secs = state.env_reveal_remaining_secs(&row.key);
    let revealed = reveal_remaining_secs.is_some();

    let mut lines = Vec::new();
    lines.push(kv_line("KEY", row.key.clone(), theme::label()));

    let (tag, tag_style) = source_tag(row.source_kind);
    lines.push(kv_line(
        "ORIGIN",
        format!("{tag} ({})", row.source_kind.label()),
        tag_style,
    ));

    if let Some(src) = &row.source {
        lines.push(kv_line(
            "SOURCE",
            shorten_path(src, width.saturating_sub(12)),
            theme::muted(),
        ));
    }
    if let Some(tool) = &row.tool {
        lines.push(kv_line("TOOL", format!("{} {tool}", glyphs::TOOL), theme::accent()));
    }

    let profile = state.profile.as_deref().unwrap_or("default");
    lines.push(kv_line("PROFILE", format!("MISE_ENV={profile}"), theme::muted()));

    if row.sensitive {
        let sec_line = if let Some(secs) = reveal_remaining_secs {
            Line::from(vec![
                Span::styled(format!("{:<10} ", "SECURITY"), theme::muted().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{} REVEALED — auto-hides in {secs}s ", glyphs::UNLOCK),
                    theme::danger().add_modifier(Modifier::BOLD),
                ),
                Span::styled("(press S to mask now)", theme::muted()),
            ])
        } else {
            let reason = if row.officially_redacted {
                "mise --redacted"
            } else {
                "sensitive key heuristic"
            };
            Line::from(vec![
                Span::styled(format!("{:<10} ", "SECURITY"), theme::muted().add_modifier(Modifier::BOLD)),
                Span::styled(
                    format!("{} MASKED ({reason}) ", glyphs::LOCK),
                    theme::warning().add_modifier(Modifier::BOLD),
                ),
                Span::styled("— press S to reveal for 30s", theme::muted()),
            ])
        };
        lines.push(sec_line);
    }

    lines.push(Line::from(""));
    lines.push(section_divider("VALUE", width));

    let display_val = row.display_value(revealed);
    if row.sensitive && !revealed {
        lines.push(Line::from(Span::styled(
            format!("  {display_val}"),
            theme::warning(),
        )));
    } else if display_val.is_empty() {
        lines.push(Line::from(Span::styled("  (empty string)", theme::muted())));
    } else {
        // Safely split multi-line values on `\n` so PEMs, JSON, and scripts never collapse.
        let val_lines: Vec<&str> = display_val.split('\n').collect();
        let show_line_nums = val_lines.len() > 1;
        for (idx, part) in val_lines.iter().enumerate() {
            let clean = part.trim_end_matches('\r');
            if show_line_nums {
                lines.push(Line::from(vec![
                    Span::styled(format!("{:>3} │ ", idx + 1), theme::subtle()),
                    Span::styled(clean.to_string(), theme::primary()),
                ]));
            } else {
                lines.push(Line::from(Span::styled(
                    format!("  {clean}"),
                    theme::primary(),
                )));
            }
        }
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  Actions: ", theme::label()),
        Span::styled("e: edit  a: add new  d: unset  S: reveal/mask  y: copy value  Y: copy KEY=VAL", theme::muted()),
    ]));
    lines
}

fn build_paths_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let entries = &state.env_vm.path_entries;
    let missing = entries.iter().filter(|p| !p.exists).count();
    let dups = entries.iter().filter(|p| p.is_duplicate).count();

    let mut lines = vec![
        section_divider(
            &format!("PATH INSPECTOR ({} entries • {} missing • {} dup)", entries.len(), missing, dups),
            width,
        ),
    ];

    if entries.is_empty() {
        lines.extend(empty_state_lines(
            "No PATH Entries Loaded",
            "Run `R` to reload `mise env --paths`.",
            None,
        ));
        return lines;
    }

    let path_budget = width.saturating_sub(24).max(16);
    for (idx, entry) in entries.iter().enumerate() {
        let (status_glyph, status_style, tag) = if !entry.exists {
            (glyphs::FAIL, theme::danger(), "MISSING")
        } else if !entry.is_dir {
            (glyphs::WARN, theme::warning(), "FILE   ")
        } else {
            (glyphs::OK, theme::success(), "OK     ")
        };

        let mut spans = vec![
            Span::styled(format!("{:>3} ", idx + 1), theme::subtle()),
            Span::styled(format!("{status_glyph} "), status_style),
            Span::styled(format!("{tag} "), status_style),
            Span::styled(
                shorten_path(&entry.display, path_budget),
                if entry.exists {
                    theme::primary()
                } else {
                    theme::muted()
                },
            ),
        ];
        if entry.is_duplicate {
            spans.push(Span::styled(" [DUP]", theme::warning()));
        }
        lines.push(Line::from(spans));
    }

    lines
}

fn build_vars_lines(state: &AppState, width: usize) -> Vec<Line<'static>> {
    let vars = state.env_vm.filtered_vars(&state.filters.env);
    let mut lines = vec![section_divider(
        &format!("TEMPLATE VARS (`[vars]` • {})", vars.len()),
        width,
    )];

    if vars.is_empty() {
        lines.extend(empty_state_lines(
            "No `[vars]` Defined",
            "Define reusable template variables under `[vars]` in `mise.toml`.",
            Some("Example: [vars] app_env = \"staging\""),
        ));
        return lines;
    }

    lines.push(Line::from(vec![
        Span::styled(
            format!("  {} ", pad_display_width("VAR", 22)),
            theme::muted().add_modifier(Modifier::BOLD),
        ),
        Span::styled("VALUE", theme::muted().add_modifier(Modifier::BOLD)),
    ]));

    for (k, v) in vars {
        let val_str = v
            .as_str()
            .map(ToOwned::to_owned)
            .unwrap_or_else(|| v.to_string());
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", glyphs::VAR), theme::purple()),
            Span::styled(format!("{} ", pad_display_width(k, 20)), theme::accent()),
            Span::styled(val_str, theme::primary()),
        ]));
    }
    lines
}

fn env_row_icon(row: &EnvRow, revealed: bool) -> (&'static str, ratatui::style::Style) {
    if row.sensitive {
        if revealed {
            (glyphs::UNLOCK, theme::danger())
        } else {
            (glyphs::LOCK, theme::warning())
        }
    } else if row.source_kind == EnvSourceKind::ToolInjected {
        (glyphs::TOOL, theme::accent())
    } else {
        (glyphs::VAR, theme::muted())
    }
}

fn source_tag(kind: EnvSourceKind) -> (&'static str, ratatui::style::Style) {
    match kind {
        EnvSourceKind::Project => ("PROJ", theme::accent()),
        EnvSourceKind::Local => ("LOCAL", theme::warning()),
        EnvSourceKind::Global => ("GLOB", theme::purple()),
        EnvSourceKind::ToolInjected => ("TOOL", theme::success()),
        EnvSourceKind::System => ("SYS", theme::muted()),
    }
}
