use ratatui::Frame;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, List, ListItem, ListState, Paragraph, Wrap};
use unicode_width::UnicodeWidthStr;

use crate::action::{Domain, Panel};
use crate::app::keymap::{KeyScope, help_entries, palette_items};
use crate::app::state::{
    AppState, ArgField, ConfirmAction, GenerateItem, MenuItem, Overlay, RegistryStep, TextInput,
};
use crate::model::env::EnvRow;
use crate::model::fuzzy::fuzzy_match;
use crate::ui::layout::LayoutMap;
use crate::ui::theme::{self, glyphs};
use crate::ui::widgets::{
    highlighted_truncated_spans, pad_display_width, render_scrollbar, text_input_spans,
    truncate_display,
};

/// Render the active modal overlay (if any) with Clear background, adaptive sizing, and LayoutMap registration.
pub fn render(frame: &mut Frame, state: &AppState, layout_map: &mut LayoutMap) {
    match &state.overlay {
        Overlay::None => {}
        Overlay::Help { query, selected } => {
            render_help_modal(frame, state, query, *selected, layout_map);
        }
        Overlay::Filter { panel, input } => {
            render_filter_bar(frame, state, *panel, input, layout_map);
        }
        Overlay::ContextMenu {
            title,
            items,
            selected,
        } => {
            render_context_menu(frame, title, items, *selected, layout_map);
        }
        Overlay::CommandPalette { query, selected } => {
            render_command_palette(frame, state, query, *selected, layout_map);
        }
        Overlay::ProfileSwitcher {
            candidates,
            input,
            selected,
        } => {
            render_profile_switcher(frame, state, candidates, input, *selected, layout_map);
        }
        Overlay::Confirm {
            title,
            detail,
            command_preview,
            action,
        } => {
            render_confirm_modal(frame, title, detail, command_preview, action, layout_map);
        }
        Overlay::TaskArgsForm {
            task_name,
            background,
            force,
            fields,
            extra_args,
            focused_field,
        } => {
            render_task_args_form(
                frame,
                task_name,
                *background,
                *force,
                fields,
                extra_args,
                *focused_field,
                layout_map,
            );
        }
        Overlay::RegistryPicker {
            query,
            selected,
            step,
        } => {
            render_registry_picker(frame, state, query, *selected, step, layout_map);
        }
        Overlay::EnvSetForm {
            key,
            value,
            target_files,
            target_idx,
            focused_field,
            is_edit,
        } => {
            render_env_set_form(
                frame,
                key,
                value,
                target_files,
                *target_idx,
                *focused_field,
                *is_edit,
                layout_map,
            );
        }
        Overlay::GenerateMenu {
            items,
            selected,
            template_name_input,
        } => {
            render_generate_menu(
                frame,
                items,
                *selected,
                template_name_input.as_ref(),
                layout_map,
            );
        }
    }
}

/// Compute a centered modal rect clamped cleanly inside `area` (never overflows even at 40x10).
fn centered_rect(area: Rect, preferred_w: u16, preferred_h: u16) -> Rect {
    let max_w = area.width.saturating_sub(2).max(1);
    let max_h = area.height.saturating_sub(2).max(1);
    let w = preferred_w.min(max_w);
    let h = preferred_h.min(max_h);
    let x = area.x.saturating_add(area.width.saturating_sub(w) / 2);
    let y = area.y.saturating_add(area.height.saturating_sub(h) / 2);
    Rect::new(x, y, w, h)
}

fn modal_block(title: &str, footer: &str, border_style: ratatui::style::Style) -> Block<'static> {
    let mut block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(border_style)
        .title_top(Line::from(vec![
            Span::styled(" ", theme::muted()),
            Span::styled(title.to_string(), theme::label()),
            Span::styled(" ", theme::muted()),
        ]));
    if !footer.is_empty() {
        block = block.title_bottom(
            Line::from(vec![
                Span::styled(" ", theme::muted()),
                Span::styled(footer.to_string(), theme::muted()),
                Span::styled(" ", theme::muted()),
            ])
            .centered(),
        );
    }
    block
}

fn scope_badge(scope: KeyScope) -> (&'static str, ratatui::style::Style) {
    match scope {
        KeyScope::Global => ("GLOBAL", theme::accent()),
        KeyScope::Panel(Panel::Tasks) => ("TASKS ", theme::success()),
        KeyScope::Panel(Panel::Tools) => ("TOOLS ", theme::warning()),
        KeyScope::Panel(Panel::Env) => ("ENV   ", theme::purple()),
        KeyScope::Panel(Panel::Config) => ("CONFIG", theme::accent()),
        KeyScope::Panel(Panel::Jobs) => ("JOBS  ", theme::muted()),
    }
}

fn render_help_modal(
    frame: &mut Frame,
    state: &AppState,
    query: &TextInput,
    selected: usize,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let modal = centered_rect(area, 86, 26);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    // Uses the exact same `help_entries` order as `app/overlay.rs` for 100% index alignment.
    let entries = help_entries(state, &query.value);
    let block = modal_block(
        &format!("? Keybindings Help ({} shown)", entries.len()),
        "Enter: run selected  ↑/↓: move  PgUp/PgDn: page  type: search  Esc: close",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let mut search_spans = vec![Span::styled("Search: ", theme::label())];
    search_spans.extend(text_input_spans(
        query,
        "type to fuzzy-filter keybindings…",
        true,
        chunks[0].width.saturating_sub(9) as usize,
    ));
    frame.render_widget(Paragraph::new(Line::from(search_spans)), chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(chunks[1].width as usize),
            theme::subtle(),
        ))),
        chunks[1],
    );

    let list_area = chunks[2];
    if entries.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No keybindings match your query.",
                theme::muted(),
            ))),
            list_area,
        );
        return;
    }

    let needle = query.value.trim();
    let desc_budget = (list_area.width as usize).saturating_sub(32).max(8);
    let items: Vec<ListItem<'static>> = entries
        .iter()
        .map(|b| {
            let (slabel, sstyle) = scope_badge(b.scope);
            let desc_indices = if needle.is_empty() {
                Vec::new()
            } else {
                fuzzy_match(needle, b.description)
                    .map(|m| m.matched_indices)
                    .unwrap_or_default()
            };
            let mut spans = vec![
                Span::styled(format!("[{slabel}] "), sstyle),
                Span::styled(format!(" {:<8} ", b.key_display), theme::key_cap()),
                Span::styled(format!(" {} ", pad_display_width(b.label, 9)), theme::primary()),
            ];
            spans.extend(highlighted_truncated_spans(
                b.description,
                &desc_indices,
                theme::muted(),
                desc_budget,
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let total = items.len();
    let clamped_sel = selected.min(total.saturating_sub(1));
    let mut ls = ListState::default();
    ls.select(Some(clamped_sel));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme::selected(true)),
        list_area,
        &mut ls,
    );
    render_scrollbar(
        frame,
        modal,
        total,
        list_area.height as usize,
        ls.offset(),
        true,
    );
}

fn render_filter_bar(
    frame: &mut Frame,
    state: &AppState,
    panel: Panel,
    input: &TextInput,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let bar_h: u16 = if area.height >= 12 { 3 } else { 1 };
    let y = if bar_h == 3 {
        area.y.saturating_add(area.height.saturating_sub(4))
    } else {
        area.y.saturating_add(area.height.saturating_sub(1))
    };
    let bar_w = area.width.saturating_sub(2).max(1);
    let rect = Rect::new(area.x.saturating_add(1), y, bar_w, bar_h);
    layout_map.overlay_area = Some(rect);
    frame.render_widget(Clear, rect);

    let count = state.current_list_len(panel);
    let prefix = format!("/ [{}] ", panel.short_title());
    let suffix = format!("  ({count} items • Enter: apply  Esc: clear)");
    let budget = (rect.width as usize)
        .saturating_sub(UnicodeWidthStr::width(prefix.as_str()) + 4)
        .max(6);

    let mut spans = vec![Span::styled(prefix, theme::label())];
    spans.extend(text_input_spans(input, "fuzzy filter…", true, budget));
    if rect.width >= 60 {
        spans.push(Span::styled(suffix, theme::muted()));
    }

    if bar_h == 3 {
        let block = Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(theme::border(true))
            .title_top(Line::from(vec![Span::styled(" Fuzzy Filter ", theme::label())]));
        frame.render_widget(Paragraph::new(Line::from(spans)).block(block), rect);
    } else {
        frame.render_widget(
            Paragraph::new(Line::from(spans)).style(theme::surface_bg()),
            rect,
        );
    }
}

fn render_context_menu(
    frame: &mut Frame,
    title: &str,
    items: &[MenuItem],
    selected: usize,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let desired_h = (items.len() as u16 + 6).clamp(8, 22);
    let modal = centered_rect(area, 76, desired_h);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    let block = modal_block(
        title,
        "Enter / key: run  j/k: move  Esc: close",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let has_preview = inner.height >= 4;
    let (list_area, preview_area) = if has_preview {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(chunks[1].width as usize),
                theme::subtle(),
            ))),
            chunks[1],
        );
        (chunks[0], Some(chunks[2]))
    } else {
        (inner, None)
    };

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No context actions available for current selection.",
                theme::muted(),
            ))),
            list_area,
        );
        return;
    }

    let desc_budget = (list_area.width as usize).saturating_sub(34).max(8);
    let rows: Vec<ListItem<'static>> = items
        .iter()
        .map(|item| {
            let key_pill = if item.key_hint.is_empty() {
                "  ·  ".to_string()
            } else {
                format!(" {:<5} ", item.key_hint)
            };
            ListItem::new(Line::from(vec![
                Span::styled(key_pill, theme::key_cap()),
                Span::styled(" ", theme::muted()),
                Span::styled(
                    format!("{} ", pad_display_width(&item.label, 24)),
                    theme::primary().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_display(&item.description, desc_budget),
                    theme::muted(),
                ),
            ]))
        })
        .collect();

    let clamped = selected.min(items.len().saturating_sub(1));
    let mut ls = ListState::default();
    ls.select(Some(clamped));
    frame.render_stateful_widget(
        List::new(rows).highlight_style(theme::selected(true)),
        list_area,
        &mut ls,
    );

    if let Some(pa) = preview_area {
        let preview = items
            .get(clamped)
            .and_then(|i| i.command_preview.as_deref())
            .unwrap_or("(in-app action)");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("$ ", theme::accent()),
                Span::styled(
                    truncate_display(preview, pa.width.saturating_sub(3) as usize),
                    theme::warning(),
                ),
            ])),
            pa,
        );
    }
}

fn render_command_palette(
    frame: &mut Frame,
    state: &AppState,
    query: &TextInput,
    selected: usize,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let modal = centered_rect(area, 82, 24);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    // Uses the exact same `palette_items` order as `app/overlay.rs` for 100% index alignment.
    let items_data = palette_items(state, &query.value);
    let block = modal_block(
        &format!(": Command Palette ({})", items_data.len()),
        "Enter: execute  ↑/↓ or Ctrl-n/p: move  Esc: close",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let has_preview = inner.height >= 6;
    let chunks = if has_preview {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Length(1),
                Constraint::Length(1),
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner)
    } else {
        Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
            .split(inner)
    };

    let mut prompt_spans = vec![Span::styled(": ", theme::label())];
    prompt_spans.extend(text_input_spans(
        query,
        "type any action, panel, or mise command…",
        true,
        chunks[0].width.saturating_sub(3) as usize,
    ));
    frame.render_widget(Paragraph::new(Line::from(prompt_spans)), chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(chunks[1].width as usize),
            theme::subtle(),
        ))),
        chunks[1],
    );

    let list_area = chunks[2];
    if items_data.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No commands matched.",
                theme::muted(),
            ))),
            list_area,
        );
        return;
    }

    let needle = query.value.trim();
    let desc_budget = (list_area.width as usize).saturating_sub(36).max(8);
    let rows: Vec<ListItem<'static>> = items_data
        .iter()
        .map(|item| {
            let label_indices = if needle.is_empty() {
                Vec::new()
            } else {
                fuzzy_match(needle, &item.label)
                    .map(|m| m.matched_indices)
                    .unwrap_or_default()
            };
            let key_tag = if item.key_hint.is_empty() {
                "  ·  ".to_string()
            } else {
                format!(" {:<6} ", item.key_hint)
            };
            let mut spans = vec![
                Span::styled(key_tag, theme::key_cap()),
                Span::raw(" "),
            ];
            spans.extend(highlighted_truncated_spans(
                &item.label,
                &label_indices,
                theme::primary().add_modifier(Modifier::BOLD),
                24,
            ));
            let label_w = UnicodeWidthStr::width(truncate_display(&item.label, 24).as_str());
            if label_w < 24 {
                spans.push(Span::raw(" ".repeat(24 - label_w)));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(
                truncate_display(&item.description, desc_budget),
                theme::muted(),
            ));
            ListItem::new(Line::from(spans))
        })
        .collect();

    let total = rows.len();
    let clamped = selected.min(total.saturating_sub(1));
    let mut ls = ListState::default();
    ls.select(Some(clamped));
    frame.render_stateful_widget(
        List::new(rows).highlight_style(theme::selected(true)),
        list_area,
        &mut ls,
    );
    render_scrollbar(
        frame,
        modal,
        total,
        list_area.height as usize,
        ls.offset(),
        true,
    );

    if has_preview {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(chunks[3].width as usize),
                theme::subtle(),
            ))),
            chunks[3],
        );
        let preview = items_data
            .get(clamped)
            .and_then(|i| i.command_preview.as_deref())
            .unwrap_or("(in-app action)");
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("$ ", theme::accent()),
                Span::styled(
                    truncate_display(preview, chunks[4].width.saturating_sub(3) as usize),
                    theme::warning(),
                ),
            ])),
            chunks[4],
        );
    }
}

fn render_profile_switcher(
    frame: &mut Frame,
    state: &AppState,
    candidates: &[String],
    input: &TextInput,
    selected: usize,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let modal = centered_rect(area, 64, 16);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    let current = state.profile.as_deref().unwrap_or("default");
    let block = modal_block(
        &format!("E Switch MISE_ENV Profile (current: {current})"),
        "Enter: apply  ↑/↓: pick  type: custom (e.g. dev,local)  Esc: cancel",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
        .split(inner);

    let mut in_spans = vec![Span::styled(
        "MISE_ENV=",
        theme::warning().add_modifier(Modifier::BOLD),
    )];
    in_spans.extend(text_input_spans(
        input,
        "leave empty to use selected candidate, or type custom…",
        true,
        chunks[0].width.saturating_sub(10) as usize,
    ));
    frame.render_widget(Paragraph::new(Line::from(in_spans)), chunks[0]);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            "─".repeat(chunks[1].width as usize),
            theme::subtle(),
        ))),
        chunks[1],
    );

    // Render `candidates` in exact index order used by `app/overlay.rs`.
    let items: Vec<ListItem<'static>> = candidates
        .iter()
        .map(|cand| {
            let is_cur = (cand == "default" && state.profile.is_none())
                || state.profile.as_deref() == Some(cand.as_str());
            let marker = if is_cur { glyphs::OK } else { " " };
            ListItem::new(Line::from(vec![
                Span::styled(format!(" {marker} "), theme::success()),
                Span::styled(cand.clone(), theme::primary()),
                if is_cur {
                    Span::styled("  (active)", theme::muted())
                } else {
                    Span::raw("")
                },
            ]))
        })
        .collect();

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "Type a profile name and press Enter.",
                theme::muted(),
            ))),
            chunks[2],
        );
        return;
    }

    let clamped = selected.min(items.len().saturating_sub(1));
    let mut ls = ListState::default();
    ls.select(Some(clamped));
    frame.render_stateful_widget(
        List::new(items).highlight_style(theme::selected(true)),
        chunks[2],
        &mut ls,
    );
}

fn render_confirm_modal(
    frame: &mut Frame,
    title: &str,
    detail: &str,
    command_preview: &str,
    action: &ConfirmAction,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let modal = centered_rect(area, 74, 12);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    let is_destructive = matches!(action, ConfirmAction::QuitWithRunningJobs)
        || command_preview.contains("prune")
        || command_preview.contains("uninstall")
        || command_preview.contains("unuse")
        || command_preview.contains("unset")
        || command_preview.contains("untrust");

    let border_style = if is_destructive {
        theme::danger()
    } else {
        theme::warning()
    };

    let block = modal_block(
        &format!("{} {title}", glyphs::WARN),
        "[Enter / y] Confirm   [Esc / n] Cancel",
        border_style,
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height == 0 || inner.width == 0 {
        return;
    }

    let mut lines = vec![Line::from(Span::styled(detail.to_string(), theme::primary()))];
    if !command_preview.trim().is_empty() {
        lines.push(Line::from(""));
        lines.push(Line::from(Span::styled(
            "Exact command to execute:",
            theme::muted().add_modifier(Modifier::BOLD),
        )));
        lines.push(Line::from(vec![
            Span::styled(" $ ", theme::accent().add_modifier(Modifier::BOLD)),
            Span::styled(
                command_preview.to_string(),
                theme::warning().add_modifier(Modifier::BOLD),
            ),
        ]));
    }

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

#[allow(clippy::too_many_arguments)]
fn render_task_args_form(
    frame: &mut Frame,
    task_name: &str,
    background: bool,
    force: bool,
    fields: &[ArgField],
    extra_args: &TextInput,
    focused_field: usize,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let desired_h = (fields.len() as u16 * 2 + 10).clamp(11, 24);
    let modal = centered_rect(area, 78, desired_h);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    let mode_tag = if background { "BG" } else { "FG" };
    let force_tag = if force { " --force" } else { "" };
    let block = modal_block(
        &format!("Run Task `{task_name}` [{mode_tag}{force_tag}]"),
        "Tab: fields  Ctrl-b: bg/fg  Ctrl-f: --force  Enter: run  Esc: cancel",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 10 {
        return;
    }

    let mut lines = Vec::new();
    let val_budget = (inner.width as usize).saturating_sub(24).max(8);

    for (idx, field) in fields.iter().enumerate() {
        let is_focused = idx == focused_field;
        let req = if field.required { "*" } else { " " };
        let label = field.flag.as_deref().unwrap_or(&field.name);
        let mut row_spans = vec![
            Span::styled(
                if is_focused { "▸ " } else { "  " },
                if is_focused { theme::label() } else { theme::muted() },
            ),
            Span::styled(
                format!("{}{:<16} ", req, truncate_display(label, 16)),
                if field.required {
                    theme::warning().add_modifier(Modifier::BOLD)
                } else {
                    theme::accent()
                },
            ),
            Span::styled("[", theme::subtle()),
        ];
        row_spans.extend(text_input_spans(
            &field.input,
            if field.help.is_empty() {
                "value"
            } else {
                &field.help
            },
            is_focused,
            val_budget,
        ));
        row_spans.push(Span::styled("]", theme::subtle()));
        lines.push(Line::from(row_spans));
    }

    let extra_focused = focused_field >= fields.len();
    let mut extra_spans = vec![
        Span::styled(
            if extra_focused { "▸ " } else { "  " },
            if extra_focused { theme::label() } else { theme::muted() },
        ),
        Span::styled(format!(" {:<16} ", "Extra args"), theme::primary()),
        Span::styled("[", theme::subtle()),
    ];
    extra_spans.extend(text_input_spans(
        extra_args,
        "optional extra CLI args…",
        extra_focused,
        val_budget,
    ));
    extra_spans.push(Span::styled("]", theme::subtle()));
    lines.push(Line::from(extra_spans));

    let mut cmd_parts = vec!["mise".to_string(), "run".to_string()];
    if force {
        cmd_parts.push("--force".to_string());
    }
    cmd_parts.push(task_name.to_string());
    for f in fields {
        let v = f.input.value.trim();
        if !v.is_empty() {
            if let Some(flag) = &f.flag {
                cmd_parts.push(flag.clone());
            }
            cmd_parts.push(v.to_string());
        }
    }
    if !extra_args.value.trim().is_empty() {
        cmd_parts.push(extra_args.value.trim().to_string());
    }

    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  $ ", theme::accent()),
        Span::styled(cmd_parts.join(" "), theme::warning()),
    ]));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_registry_picker(
    frame: &mut Frame,
    state: &AppState,
    query: &TextInput,
    selected: usize,
    step: &RegistryStep,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let modal = centered_rect(area, 86, 26);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    match step {
        RegistryStep::SearchTool => {
            let total_reg = if !state.tools_vm.registry.is_empty() {
                state.tools_vm.registry.len()
            } else {
                state.registry.len()
            };
            let matches = state.filtered_registry(&query.value);
            let loading = state.loading_domains.contains(&Domain::Registry);

            let title = if loading && total_reg == 0 {
                format!("{} Add Tool — Loading Mise Registry…", state.spinner_char())
            } else {
                format!(
                    "a Mise Registry ({}/{} tools) — Step 1/3",
                    matches.len(),
                    total_reg
                )
            };
            let block = modal_block(
                &title,
                "Enter: pick version  ↑/↓: move  PgUp/PgDn: page  type: fuzzy search  Esc: close",
                theme::border(true),
            );
            let inner = block.inner(modal);
            frame.render_widget(block, modal);
            if inner.height < 3 || inner.width < 12 {
                return;
            }

            let has_detail = inner.height >= 7;
            let chunks = if has_detail {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                        Constraint::Length(1),
                        Constraint::Length(2),
                    ])
                    .split(inner)
            } else {
                Layout::default()
                    .direction(Direction::Vertical)
                    .constraints([
                        Constraint::Length(1),
                        Constraint::Length(1),
                        Constraint::Min(1),
                    ])
                    .split(inner)
            };

            let mut search_spans = vec![Span::styled("Search: ", theme::label())];
            search_spans.extend(text_input_spans(
                query,
                "name, alias, binary, backend, or description…",
                true,
                chunks[0].width.saturating_sub(9) as usize,
            ));
            frame.render_widget(Paragraph::new(Line::from(search_spans)), chunks[0]);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "─".repeat(chunks[1].width as usize),
                    theme::subtle(),
                ))),
                chunks[1],
            );

            let list_area = chunks[2];
            if matches.is_empty() {
                let msg = if loading {
                    format!("{} Fetching `mise registry --json`…", state.spinner_char())
                } else {
                    "No registry tools matched your query.".to_string()
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(msg, theme::muted()))),
                    list_area,
                );
                return;
            }

            let total = matches.len();
            let clamped = selected.min(total.saturating_sub(1));
            let window_start = clamped.saturating_sub(50);
            let window_end = (window_start + 150).min(total);
            let slice = &matches[window_start..window_end];

            let w = list_area.width as usize;
            let items: Vec<ListItem<'static>> = slice
                .iter()
                .map(|m| {
                    let backend = m
                        .entry
                        .backends
                        .first()
                        .map(String::as_str)
                        .unwrap_or("");
                    let backend_budget = (w / 2).clamp(12, 34);
                    let short_budget = w.saturating_sub(backend_budget + 6).clamp(10, 24);
                    let mut spans = vec![Span::raw(" ")];
                    spans.extend(highlighted_truncated_spans(
                        &m.entry.short,
                        &m.match_indices,
                        theme::primary().add_modifier(Modifier::BOLD),
                        short_budget,
                    ));
                    let short_w = UnicodeWidthStr::width(
                        truncate_display(&m.entry.short, short_budget).as_str(),
                    );
                    if short_w < short_budget {
                        spans.push(Span::raw(" ".repeat(short_budget - short_w)));
                    }
                    spans.push(Span::styled("  ", theme::muted()));
                    spans.push(Span::styled(
                        truncate_display(backend, backend_budget),
                        theme::purple(),
                    ));
                    ListItem::new(Line::from(spans))
                })
                .collect();

            let mut ls = ListState::default();
            ls.select(Some(clamped.saturating_sub(window_start)));
            frame.render_stateful_widget(
                List::new(items).highlight_style(theme::selected(true)),
                list_area,
                &mut ls,
            );
            render_scrollbar(
                frame,
                modal,
                total,
                list_area.height as usize,
                clamped,
                true,
            );

            if has_detail {
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(
                        "─".repeat(chunks[3].width as usize),
                        theme::subtle(),
                    ))),
                    chunks[3],
                );
                if let Some(sel_match) = matches.get(clamped) {
                    let e = &sel_match.entry;
                    let desc = e.description.as_deref().unwrap_or("(no description)");
                    let mut extra = Vec::new();
                    if !e.aliases.is_empty() {
                        extra.push(format!("aliases: {}", e.aliases.join(", ")));
                    }
                    if !e.bins.is_empty() {
                        extra.push(format!("bins: {}", e.bins.join(", ")));
                    }
                    let detail_lines = vec![
                        Line::from(vec![
                            Span::styled(format!("{} ", e.short), theme::label()),
                            Span::styled(
                                truncate_display(
                                    desc,
                                    chunks[4].width.saturating_sub(e.short.len() as u16 + 2)
                                        as usize,
                                ),
                                theme::primary(),
                            ),
                        ]),
                        Line::from(Span::styled(
                            truncate_display(&extra.join("  •  "), chunks[4].width as usize),
                            theme::muted(),
                        )),
                    ];
                    frame.render_widget(Paragraph::new(detail_lines), chunks[4]);
                }
            }
        }
        RegistryStep::PickVersion {
            tool,
            versions,
            loading,
        } => {
            let block = modal_block(
                &format!(
                    "Add `{tool}` — Step 2/3: Pick Version{}",
                    if *loading { " (loading…)" } else { "" }
                ),
                "Enter: pick scope  ↑/↓: move  type: filter/custom version  Esc: back",
                theme::border(true),
            );
            let inner = block.inner(modal);
            frame.render_widget(block, modal);
            if inner.height < 2 || inner.width < 10 {
                return;
            }

            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(1), Constraint::Length(1), Constraint::Min(1)])
                .split(inner);

            let mut filter_spans = vec![Span::styled("Version: ", theme::label())];
            filter_spans.extend(text_input_spans(
                query,
                "filter versions or enter custom tag…",
                true,
                chunks[0].width.saturating_sub(10) as usize,
            ));
            frame.render_widget(Paragraph::new(Line::from(filter_spans)), chunks[0]);
            frame.render_widget(
                Paragraph::new(Line::from(Span::styled(
                    "─".repeat(chunks[1].width as usize),
                    theme::subtle(),
                ))),
                chunks[1],
            );

            // Match `app/overlay.rs` filtering 1:1.
            let needle = query.value.trim();
            let filtered: Vec<&String> = if needle.is_empty() {
                versions.iter().collect()
            } else {
                versions.iter().filter(|v| v.contains(needle)).collect()
            };

            if filtered.is_empty() {
                let msg = if *loading {
                    format!(
                        "  {} Fetching available versions via `mise ls-remote {tool}`…",
                        state.spinner_char()
                    )
                } else {
                    format!("  Press Enter to use custom version `{needle}`.")
                };
                frame.render_widget(
                    Paragraph::new(Line::from(Span::styled(msg, theme::accent()))),
                    chunks[2],
                );
                return;
            }

            let total = filtered.len();
            let clamped = selected.min(total.saturating_sub(1));
            let window_start = clamped.saturating_sub(50);
            let window_end = (window_start + 150).min(total);
            let items: Vec<ListItem<'static>> = filtered[window_start..window_end]
                .iter()
                .map(|v| {
                    let style = if v.as_str() == "latest" {
                        theme::success().add_modifier(Modifier::BOLD)
                    } else {
                        theme::primary()
                    };
                    ListItem::new(Line::from(vec![
                        Span::raw("  "),
                        Span::styled((*v).clone(), style),
                    ]))
                })
                .collect();

            let mut ls = ListState::default();
            ls.select(Some(clamped.saturating_sub(window_start)));
            frame.render_stateful_widget(
                List::new(items).highlight_style(theme::selected(true)),
                chunks[2],
                &mut ls,
            );
            render_scrollbar(
                frame,
                modal,
                total,
                chunks[2].height as usize,
                clamped,
                true,
            );
        }
        RegistryStep::PickScope { tool, version } => {
            let block = modal_block(
                &format!("Add `{tool}@{version}` — Step 3/3: Target Scope"),
                "Enter: confirm install  ↑/↓: move  Esc: back",
                theme::border(true),
            );
            let inner = block.inner(modal);
            frame.render_widget(block, modal);
            if inner.height == 0 || inner.width == 0 {
                return;
            }

            let options = [
                ("Project (mise.toml)", format!("mise use {tool}@{version}")),
                (
                    "Global (~/.config/mise/config.toml)",
                    format!("mise use -g {tool}@{version}"),
                ),
            ];
            let items: Vec<ListItem<'static>> = options
                .iter()
                .map(|(label, cmd)| {
                    ListItem::new(Line::from(vec![
                        Span::styled(
                            format!("  {:<36} ", label),
                            theme::primary().add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(format!("$ {cmd}"), theme::warning()),
                    ]))
                })
                .collect();

            let clamped = selected.min(1);
            let mut ls = ListState::default();
            ls.select(Some(clamped));
            frame.render_stateful_widget(
                List::new(items).highlight_style(theme::selected(true)),
                inner,
                &mut ls,
            );
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn render_env_set_form(
    frame: &mut Frame,
    key: &TextInput,
    value: &TextInput,
    target_files: &[String],
    target_idx: usize,
    focused_field: usize,
    is_edit: bool,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let modal = centered_rect(area, 72, 13);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    let title = if is_edit {
        "Edit Environment Variable (`mise set`)"
    } else {
        "Add Environment Variable (`mise set`)"
    };
    let block = modal_block(
        title,
        "Tab: next field  ←/→: target file  Enter: apply  Esc: cancel",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 3 || inner.width < 12 {
        return;
    }

    let val_budget = (inner.width as usize).saturating_sub(18).max(8);
    let target_file = target_files
        .get(target_idx)
        .map(String::as_str)
        .unwrap_or("mise.toml");

    let mut key_row = vec![
        Span::styled(if focused_field == 0 { "▸ " } else { "  " }, theme::label()),
        Span::styled(format!("{:<10} ", "KEY"), theme::accent()),
        Span::styled("[", theme::subtle()),
    ];
    key_row.extend(text_input_spans(
        key,
        "ENV_VAR_NAME",
        focused_field == 0,
        val_budget,
    ));
    key_row.push(Span::styled("]", theme::subtle()));

    let mut val_row = vec![
        Span::styled(if focused_field == 1 { "▸ " } else { "  " }, theme::label()),
        Span::styled(format!("{:<10} ", "VALUE"), theme::accent()),
        Span::styled("[", theme::subtle()),
    ];
    val_row.extend(text_input_spans(
        value,
        "value",
        focused_field == 1,
        val_budget,
    ));
    val_row.push(Span::styled("]", theme::subtle()));

    let target_row = Line::from(vec![
        Span::styled(if focused_field == 2 { "▸ " } else { "  " }, theme::label()),
        Span::styled(format!("{:<10} ", "TARGET"), theme::accent()),
        Span::styled(
            format!("◀ {target_file} ▶"),
            if focused_field == 2 {
                theme::warning().add_modifier(Modifier::BOLD)
            } else {
                theme::primary()
            },
        ),
    ]);

    let mut lines = vec![Line::from(key_row), Line::from(val_row), target_row];

    if EnvRow::is_sensitive_key(&key.value) {
        lines.push(Line::from(vec![
            Span::styled(format!("  {} ", glyphs::LOCK), theme::warning()),
            Span::styled(
                "Sensitive key detected — consider writing to `mise.local.toml` (gitignored).",
                theme::warning(),
            ),
        ]));
    }

    let preview_val = if EnvRow::is_sensitive_key(&key.value) && !value.value.is_empty() {
        "••••••••"
    } else {
        &value.value
    };
    let cmd_preview = if target_file == "mise.toml" {
        format!("mise set {}={preview_val}", key.value)
    } else {
        format!("mise set --file {target_file} {}={preview_val}", key.value)
    };
    lines.push(Line::from(""));
    lines.push(Line::from(vec![
        Span::styled("  $ ", theme::accent()),
        Span::styled(cmd_preview, theme::warning()),
    ]));

    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn render_generate_menu(
    frame: &mut Frame,
    items: &[GenerateItem],
    selected: usize,
    template_name_input: Option<&(String, TextInput)>,
    layout_map: &mut LayoutMap,
) {
    let area = frame.area();
    let desired_h = (items.len() as u16 + 7).clamp(10, 22);
    let modal = centered_rect(area, 78, desired_h);
    layout_map.overlay_area = Some(modal);
    frame.render_widget(Clear, modal);

    let block = modal_block(
        "g Generate & Scaffold (`mise generate` + Templates)",
        "Enter: run  j/k: move  Esc: close",
        theme::border(true),
    );
    let inner = block.inner(modal);
    frame.render_widget(block, modal);
    if inner.height < 2 || inner.width < 10 {
        return;
    }

    if let Some((tpl_name, input)) = template_name_input {
        let mut in_spans = vec![Span::styled("  Project name: [", theme::accent())];
        in_spans.extend(text_input_spans(
            input,
            "my-new-project",
            true,
            inner.width.saturating_sub(20) as usize,
        ));
        in_spans.push(Span::styled("]", theme::accent()));

        let lines = vec![
            Line::from(vec![
                Span::styled("  Scaffold template: ", theme::muted()),
                Span::styled(tpl_name.clone(), theme::label()),
            ]),
            Line::from(""),
            Line::from(in_spans),
            Line::from(""),
            Line::from(vec![
                Span::styled("  $ ", theme::accent()),
                Span::styled(
                    format!("mise run new {tpl_name} {}", input.value),
                    theme::warning(),
                ),
            ]),
        ];
        frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
        return;
    }

    let (list_area, preview_area) = if inner.height >= 4 {
        let chunks = Layout::default()
            .direction(Direction::Vertical)
            .constraints([
                Constraint::Min(1),
                Constraint::Length(1),
                Constraint::Length(1),
            ])
            .split(inner);
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "─".repeat(chunks[1].width as usize),
                theme::subtle(),
            ))),
            chunks[1],
        );
        (chunks[0], Some(chunks[2]))
    } else {
        (inner, None)
    };

    if items.is_empty() {
        frame.render_widget(
            Paragraph::new(Line::from(Span::styled(
                "No generators or workspace templates available.",
                theme::muted(),
            ))),
            list_area,
        );
        return;
    }

    let desc_budget = (list_area.width as usize).saturating_sub(32).max(8);
    let rows: Vec<ListItem<'static>> = items
        .iter()
        .map(|item| {
            ListItem::new(Line::from(vec![
                Span::raw("  "),
                Span::styled(
                    format!("{} ", pad_display_width(&item.label, 26)),
                    theme::primary().add_modifier(Modifier::BOLD),
                ),
                Span::styled(
                    truncate_display(&item.description, desc_budget),
                    theme::muted(),
                ),
            ]))
        })
        .collect();

    let clamped = selected.min(items.len().saturating_sub(1));
    let mut ls = ListState::default();
    ls.select(Some(clamped));
    frame.render_stateful_widget(
        List::new(rows).highlight_style(theme::selected(true)),
        list_area,
        &mut ls,
    );

    if let Some(pa) = preview_area
        && let Some(item) = items.get(clamped)
    {
        frame.render_widget(
            Paragraph::new(Line::from(vec![
                Span::styled("$ ", theme::accent()),
                Span::styled(
                    truncate_display(&item.command_preview, pa.width.saturating_sub(3) as usize),
                    theme::warning(),
                ),
            ])),
            pa,
        );
    }
}
