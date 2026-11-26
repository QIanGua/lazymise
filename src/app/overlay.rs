use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

use crate::action::{Domain, ToastLevel};
use crate::app::keymap::{help_entries, palette_items};
use crate::app::reducer::{
    apply_action, execute_confirmed, jump_to_task, launch_task_execution, trigger_hot_reload,
};
use crate::app::state::{
    AppState, ConfirmAction, GenerateKind, Overlay, RegistryStep, TextInput,
};
use crate::mise::loader::MiseLoader;
use crate::runner::{BackgroundRunner, split_shell_words};

/// Handle keyboard input when an `Overlay` is active. Returns `true` if handled.
pub fn handle_overlay_key(
    state: &mut AppState,
    key: KeyEvent,
    loader: &MiseLoader,
    runner: &BackgroundRunner,
) -> bool {
    if matches!(state.overlay, Overlay::None) {
        return false;
    }

    // Ctrl-C inside any overlay closes it cleanly (never traps the user)
    if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') {
        state.overlay = Overlay::None;
        state.dirty = true;
        return true;
    }

    let overlay = std::mem::take(&mut state.overlay);
    match overlay {
        Overlay::None => return false,
        Overlay::Help {
            mut query,
            mut selected,
        } => {
            let entries = help_entries(state, &query.value);
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Char('?') if query.value.is_empty() => {}
                KeyCode::Down => {
                    if !entries.is_empty() {
                        selected = (selected + 1).min(entries.len() - 1);
                    }
                    state.overlay = Overlay::Help { query, selected };
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    state.overlay = Overlay::Help { query, selected };
                }
                KeyCode::PageDown => {
                    if !entries.is_empty() {
                        selected = (selected + 8).min(entries.len() - 1);
                    }
                    state.overlay = Overlay::Help { query, selected };
                }
                KeyCode::PageUp => {
                    selected = selected.saturating_sub(8);
                    state.overlay = Overlay::Help { query, selected };
                }
                KeyCode::Enter => {
                    if let Some(binding) = entries.get(selected) {
                        let action = binding.action.clone();
                        apply_action(state, action, loader, runner);
                    }
                }
                _ => {
                    if handle_text_input(&mut query, key) {
                        selected = 0;
                    }
                    state.overlay = Overlay::Help { query, selected };
                }
            }
        }
        Overlay::Filter { panel, mut input } => match key.code {
            KeyCode::Enter => {
                *state.filters.get_mut(panel) = input.value;
                state.rebuild_panel(panel);
            }
            KeyCode::Esc => {
                state.filters.get_mut(panel).clear();
                state.rebuild_panel(panel);
                state.push_toast(ToastLevel::Info, "Filter cleared");
            }
            _ => {
                if handle_text_input(&mut input, key) {
                    *state.filters.get_mut(panel) = input.value.clone();
                    state.rebuild_panel(panel);
                }
                state.overlay = Overlay::Filter { panel, input };
            }
        },
        Overlay::ContextMenu {
            title,
            items,
            mut selected,
        } => match key.code {
            KeyCode::Esc | KeyCode::Char('x') | KeyCode::Char('q') => {}
            KeyCode::Char('j') | KeyCode::Down => {
                if !items.is_empty() {
                    selected = (selected + 1) % items.len();
                }
                state.overlay = Overlay::ContextMenu {
                    title,
                    items,
                    selected,
                };
            }
            KeyCode::Char('k') | KeyCode::Up => {
                if !items.is_empty() {
                    selected = if selected == 0 {
                        items.len() - 1
                    } else {
                        selected - 1
                    };
                }
                state.overlay = Overlay::ContextMenu {
                    title,
                    items,
                    selected,
                };
            }
            KeyCode::Enter => {
                if let Some(item) = items.get(selected) {
                    let action = item.action.clone();
                    apply_action(state, action, loader, runner);
                }
            }
            KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                let s = c.to_string();
                if let Some(item) = items.iter().find(|i| i.key_hint == s) {
                    let action = item.action.clone();
                    apply_action(state, action, loader, runner);
                } else {
                    state.overlay = Overlay::ContextMenu {
                        title,
                        items,
                        selected,
                    };
                }
            }
            _ => {
                state.overlay = Overlay::ContextMenu {
                    title,
                    items,
                    selected,
                };
            }
        },
        Overlay::CommandPalette {
            mut query,
            mut selected,
        } => {
            let items = palette_items(state, &query.value);
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Down => {
                    if !items.is_empty() {
                        selected = (selected + 1).min(items.len() - 1);
                    }
                    state.overlay = Overlay::CommandPalette { query, selected };
                }
                KeyCode::Up => {
                    selected = selected.saturating_sub(1);
                    state.overlay = Overlay::CommandPalette { query, selected };
                }
                KeyCode::Char('n') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if !items.is_empty() {
                        selected = (selected + 1) % items.len();
                    }
                    state.overlay = Overlay::CommandPalette { query, selected };
                }
                KeyCode::Char('p') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    if !items.is_empty() {
                        selected = if selected == 0 {
                            items.len() - 1
                        } else {
                            selected - 1
                        };
                    }
                    state.overlay = Overlay::CommandPalette { query, selected };
                }
                KeyCode::Enter => {
                    if let Some(item) = items.get(selected) {
                        if let Some(task_name) = item.label.strip_prefix("Task: ") {
                            jump_to_task(state, task_name, loader);
                        } else {
                            let action = item.action.clone();
                            apply_action(state, action, loader, runner);
                        }
                    }
                }
                _ => {
                    if handle_text_input(&mut query, key) {
                        selected = 0;
                    }
                    state.overlay = Overlay::CommandPalette { query, selected };
                }
            }
        }
        Overlay::ProfileSwitcher {
            candidates,
            mut input,
            mut selected,
        } => match key.code {
            KeyCode::Esc => {}
            KeyCode::Down => {
                if !candidates.is_empty() {
                    selected = (selected + 1) % candidates.len();
                }
                state.overlay = Overlay::ProfileSwitcher {
                    candidates,
                    input,
                    selected,
                };
            }
            KeyCode::Up => {
                if !candidates.is_empty() {
                    selected = if selected == 0 {
                        candidates.len() - 1
                    } else {
                        selected - 1
                    };
                }
                state.overlay = Overlay::ProfileSwitcher {
                    candidates,
                    input,
                    selected,
                };
            }
            KeyCode::Enter => {
                let chosen = if !input.value.trim().is_empty() {
                    input.value.trim().to_string()
                } else {
                    candidates
                        .get(selected)
                        .cloned()
                        .unwrap_or_else(|| "default".to_string())
                };
                let next_profile = if chosen == "default" || chosen.is_empty() {
                    None
                } else {
                    Some(chosen.clone())
                };
                state.profile = next_profile;
                state.task_info_cache.clear();
                state.task_deps_cache.clear();
                state.task_history_cache.clear();
                state.tools_vm.details_cache.clear();
                state.requested_domains.clear();
                let label = state.profile.as_deref().unwrap_or("default").to_string();
                state.push_toast(ToastLevel::Info, format!("Switched MISE_ENV -> {label}"));
                trigger_hot_reload(state, loader, true);
            }
            _ => {
                handle_text_input(&mut input, key);
                state.overlay = Overlay::ProfileSwitcher {
                    candidates,
                    input,
                    selected,
                };
            }
        },
        Overlay::Confirm {
            title,
            detail,
            command_preview,
            action,
        } => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => {
                execute_confirmed(state, action, loader, runner);
            }
            KeyCode::Char('n') | KeyCode::Esc => {
                state.push_toast(ToastLevel::Info, "Action cancelled");
            }
            _ => {
                state.overlay = Overlay::Confirm {
                    title,
                    detail,
                    command_preview,
                    action,
                };
            }
        },
        Overlay::TaskArgsForm {
            task_name,
            mut background,
            mut force,
            mut fields,
            mut extra_args,
            mut focused_field,
        } => {
            let total_fields = fields.len() + 1; // last field is extra_args
            match key.code {
                KeyCode::Esc => {}
                KeyCode::Tab | KeyCode::Down => {
                    focused_field = (focused_field + 1) % total_fields;
                    state.overlay = Overlay::TaskArgsForm {
                        task_name,
                        background,
                        force,
                        fields,
                        extra_args,
                        focused_field,
                    };
                }
                KeyCode::BackTab | KeyCode::Up => {
                    focused_field = if focused_field == 0 {
                        total_fields - 1
                    } else {
                        focused_field - 1
                    };
                    state.overlay = Overlay::TaskArgsForm {
                        task_name,
                        background,
                        force,
                        fields,
                        extra_args,
                        focused_field,
                    };
                }
                KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    background = !background;
                    state.overlay = Overlay::TaskArgsForm {
                        task_name,
                        background,
                        force,
                        fields,
                        extra_args,
                        focused_field,
                    };
                }
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    force = !force;
                    state.overlay = Overlay::TaskArgsForm {
                        task_name,
                        background,
                        force,
                        fields,
                        extra_args,
                        focused_field,
                    };
                }
                KeyCode::Enter => {
                    // Validate required usage_spec inputs
                    if let Some(missing) = fields
                        .iter()
                        .find(|f| f.required && f.input.value.trim().is_empty())
                    {
                        state.push_toast(
                            ToastLevel::Warning,
                            format!("Required argument `{}` cannot be empty", missing.name),
                        );
                        state.overlay = Overlay::TaskArgsForm {
                            task_name,
                            background,
                            force,
                            fields,
                            extra_args,
                            focused_field,
                        };
                    } else {
                        let mut built_args = Vec::new();
                        for field in &fields {
                            let val = field.input.value.trim();
                            if val.is_empty() {
                                continue;
                            }
                            if let Some(flag) = &field.flag {
                                built_args.push(flag.clone());
                            }
                            built_args.push(val.to_string());
                        }
                        if !extra_args.value.trim().is_empty() {
                            match split_shell_words(&extra_args.value) {
                                Ok(parsed) => built_args.extend(parsed),
                                Err(err) => {
                                    state.push_toast(ToastLevel::Error, format!("{err}"));
                                    state.overlay = Overlay::TaskArgsForm {
                                        task_name,
                                        background,
                                        force,
                                        fields,
                                        extra_args,
                                        focused_field,
                                    };
                                    state.dirty = true;
                                    return true;
                                }
                            }
                        }
                        launch_task_execution(state, runner, task_name, built_args, background, force);
                    }
                }
                _ => {
                    if focused_field < fields.len() {
                        handle_text_input(&mut fields[focused_field].input, key);
                    } else {
                        handle_text_input(&mut extra_args, key);
                    }
                    state.overlay = Overlay::TaskArgsForm {
                        task_name,
                        background,
                        force,
                        fields,
                        extra_args,
                        focused_field,
                    };
                }
            }
        }
        Overlay::RegistryPicker {
            mut query,
            mut selected,
            step,
        } => match step {
            RegistryStep::SearchTool => {
                let matches = state.filtered_registry(&query.value);
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Down => {
                        if !matches.is_empty() {
                            selected = (selected + 1).min(matches.len() - 1);
                        }
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::SearchTool,
                        };
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::SearchTool,
                        };
                    }
                    KeyCode::PageDown => {
                        if !matches.is_empty() {
                            selected = (selected + 10).min(matches.len() - 1);
                        }
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::SearchTool,
                        };
                    }
                    KeyCode::PageUp => {
                        selected = selected.saturating_sub(10);
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::SearchTool,
                        };
                    }
                    KeyCode::Enter => {
                        if let Some(m) = matches.get(selected) {
                            let tool = m.entry.short.clone();
                            state.loading_domains.insert(Domain::ToolVersions);
                            loader.request_domain(
                                state.generation,
                                state.profile.clone(),
                                Domain::ToolVersions,
                                Some(tool.clone()),
                            );
                            state.overlay = Overlay::RegistryPicker {
                                query: TextInput::default(),
                                selected: 0,
                                step: RegistryStep::PickVersion {
                                    tool,
                                    versions: vec!["latest".to_string()],
                                    loading: true,
                                },
                            };
                        } else {
                            state.overlay = Overlay::RegistryPicker {
                                query,
                                selected,
                                step: RegistryStep::SearchTool,
                            };
                        }
                    }
                    _ => {
                        if handle_text_input(&mut query, key) {
                            selected = 0;
                        }
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::SearchTool,
                        };
                    }
                }
            }
            RegistryStep::PickVersion {
                tool,
                versions,
                loading,
            } => {
                let filtered: Vec<&String> = if query.value.trim().is_empty() {
                    versions.iter().collect()
                } else {
                    versions
                        .iter()
                        .filter(|v| v.contains(query.value.trim()))
                        .collect()
                };
                match key.code {
                    KeyCode::Esc => {
                        state.overlay = Overlay::RegistryPicker {
                            query: TextInput::default(),
                            selected: 0,
                            step: RegistryStep::SearchTool,
                        };
                    }
                    KeyCode::Down => {
                        if !filtered.is_empty() {
                            selected = (selected + 1).min(filtered.len() - 1);
                        }
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::PickVersion {
                                tool,
                                versions,
                                loading,
                            },
                        };
                    }
                    KeyCode::Up => {
                        selected = selected.saturating_sub(1);
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::PickVersion {
                                tool,
                                versions,
                                loading,
                            },
                        };
                    }
                    KeyCode::Enter => {
                        let chosen = filtered
                            .get(selected)
                            .map(|s| (*s).clone())
                            .or_else(|| {
                                let custom = query.value.trim();
                                (!custom.is_empty()).then(|| custom.to_string())
                            })
                            .unwrap_or_else(|| "latest".to_string());
                        state.overlay = Overlay::RegistryPicker {
                            query: TextInput::default(),
                            selected: 0,
                            step: RegistryStep::PickScope {
                                tool,
                                version: chosen,
                            },
                        };
                    }
                    _ => {
                        if handle_text_input(&mut query, key) {
                            selected = 0;
                        }
                        state.overlay = Overlay::RegistryPicker {
                            query,
                            selected,
                            step: RegistryStep::PickVersion {
                                tool,
                                versions,
                                loading,
                            },
                        };
                    }
                }
            }
            RegistryStep::PickScope { tool, version } => match key.code {
                KeyCode::Esc => {
                    state.overlay = Overlay::RegistryPicker {
                        query: TextInput::default(),
                        selected: 0,
                        step: RegistryStep::SearchTool,
                    };
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Char('j') | KeyCode::Char('k') => {
                    selected = if selected == 0 { 1 } else { 0 };
                    state.overlay = Overlay::RegistryPicker {
                        query,
                        selected,
                        step: RegistryStep::PickScope { tool, version },
                    };
                }
                KeyCode::Enter => {
                    let spec = format!("{tool}@{version}");
                    let (args, scope_desc) = if selected == 1 {
                        (vec!["use".into(), "-g".into(), spec.clone()], "global config")
                    } else {
                        (vec!["use".into(), spec.clone()], "project mise.toml")
                    };
                    let cmd_preview = state.format_mise_command(&args);
                    state.overlay = Overlay::Confirm {
                        title: format!("Add tool `{spec}` to {scope_desc}"),
                        detail: format!("Installs and pins `{spec}` in {scope_desc}."),
                        command_preview: cmd_preview,
                        action: ConfirmAction::MiseCommand {
                            args,
                            foreground: true,
                            reload_after: true,
                            label: format!("use:{spec}"),
                        },
                    };
                }
                _ => {
                    state.overlay = Overlay::RegistryPicker {
                        query,
                        selected,
                        step: RegistryStep::PickScope { tool, version },
                    };
                }
            },
        },
        Overlay::EnvSetForm {
            key: mut env_key,
            mut value,
            target_files,
            mut target_idx,
            mut focused_field,
            is_edit,
        } => match key.code {
            KeyCode::Esc => {}
            KeyCode::Tab | KeyCode::Down => {
                focused_field = (focused_field + 1) % 3;
                state.overlay = Overlay::EnvSetForm {
                    key: env_key,
                    value,
                    target_files,
                    target_idx,
                    focused_field,
                    is_edit,
                };
            }
            KeyCode::BackTab | KeyCode::Up => {
                focused_field = if focused_field == 0 {
                    2
                } else {
                    focused_field - 1
                };
                state.overlay = Overlay::EnvSetForm {
                    key: env_key,
                    value,
                    target_files,
                    target_idx,
                    focused_field,
                    is_edit,
                };
            }
            KeyCode::Left if focused_field == 2 && !target_files.is_empty() => {
                target_idx = if target_idx == 0 {
                    target_files.len() - 1
                } else {
                    target_idx - 1
                };
                state.overlay = Overlay::EnvSetForm {
                    key: env_key,
                    value,
                    target_files,
                    target_idx,
                    focused_field,
                    is_edit,
                };
            }
            KeyCode::Right | KeyCode::Char(' ')
                if focused_field == 2 && !target_files.is_empty() =>
            {
                target_idx = (target_idx + 1) % target_files.len();
                state.overlay = Overlay::EnvSetForm {
                    key: env_key,
                    value,
                    target_files,
                    target_idx,
                    focused_field,
                    is_edit,
                };
            }
            KeyCode::Enter => {
                let k = env_key.value.trim().to_string();
                let v = value.value.clone();
                if k.is_empty() || k.contains('=') || k.chars().any(char::is_whitespace) {
                    state.push_toast(
                        ToastLevel::Warning,
                        "Env key must be non-empty without spaces or `=`",
                    );
                    state.overlay = Overlay::EnvSetForm {
                        key: env_key,
                        value,
                        target_files,
                        target_idx,
                        focused_field,
                        is_edit,
                    };
                } else {
                    let target = target_files
                        .get(target_idx)
                        .cloned()
                        .unwrap_or_else(|| "mise.toml".to_string());
                    let mut args = vec!["set".to_string()];
                    if target != "mise.toml" {
                        args.push("--file".to_string());
                        args.push(target.clone());
                    }
                    args.push(format!("{k}={v}"));
                    let preview = state.format_mise_command_safe(&args);
                    state.overlay = Overlay::Confirm {
                        title: format!("Set `{k}` in `{target}`"),
                        detail: format!("Writes `{k}` to `{target}` via `mise set`."),
                        command_preview: preview,
                        action: ConfirmAction::MiseCommand {
                            args,
                            foreground: false,
                            reload_after: true,
                            label: format!("set:{k}"),
                        },
                    };
                }
            }
            _ => {
                match focused_field {
                    0 if !is_edit => {
                        handle_text_input(&mut env_key, key);
                    }
                    1 => {
                        handle_text_input(&mut value, key);
                    }
                    _ => {}
                }
                state.overlay = Overlay::EnvSetForm {
                    key: env_key,
                    value,
                    target_files,
                    target_idx,
                    focused_field,
                    is_edit,
                };
            }
        },
        Overlay::GenerateMenu {
            items,
            mut selected,
            template_name_input,
        } => {
            if let Some((template_name, mut input)) = template_name_input {
                match key.code {
                    KeyCode::Esc => {
                        state.overlay = Overlay::GenerateMenu {
                            items,
                            selected,
                            template_name_input: None,
                        };
                    }
                    KeyCode::Enter => {
                        let name = input.value.trim().to_string();
                        if name.is_empty() || name.contains('/') || name.starts_with('.') {
                            state.push_toast(
                                ToastLevel::Warning,
                                "Project name must be a non-empty single directory name",
                            );
                            state.overlay = Overlay::GenerateMenu {
                                items,
                                selected,
                                template_name_input: Some((template_name, input)),
                            };
                        } else {
                            let cmd = state.format_mise_command(&[
                                "run".into(),
                                "new".into(),
                                template_name.clone(),
                                name.clone(),
                            ]);
                            state.overlay = Overlay::Confirm {
                                title: format!("Scaffold `{template_name}` workspace"),
                                detail: format!("Creates new workspace directory `{name}`."),
                                command_preview: cmd,
                                action: ConfirmAction::ScaffoldTemplate {
                                    template: template_name,
                                    name,
                                },
                            };
                        }
                    }
                    _ => {
                        handle_text_input(&mut input, key);
                        state.overlay = Overlay::GenerateMenu {
                            items,
                            selected,
                            template_name_input: Some((template_name, input)),
                        };
                    }
                }
            } else {
                match key.code {
                    KeyCode::Esc | KeyCode::Char('q') => {}
                    KeyCode::Char('j') | KeyCode::Down => {
                        if !items.is_empty() {
                            selected = (selected + 1) % items.len();
                        }
                        state.overlay = Overlay::GenerateMenu {
                            items,
                            selected,
                            template_name_input: None,
                        };
                    }
                    KeyCode::Char('k') | KeyCode::Up => {
                        if !items.is_empty() {
                            selected = if selected == 0 {
                                items.len() - 1
                            } else {
                                selected - 1
                            };
                        }
                        state.overlay = Overlay::GenerateMenu {
                            items,
                            selected,
                            template_name_input: None,
                        };
                    }
                    KeyCode::Enter => {
                        if let Some(item) = items.get(selected).cloned() {
                            match item.kind {
                                GenerateKind::MiseGenerate { args } => {
                                    let preview = state.format_mise_command(&args);
                                    state.overlay = Overlay::Confirm {
                                        title: format!("Run `{}`", item.label),
                                        detail: item.description,
                                        command_preview: preview,
                                        action: ConfirmAction::MiseCommand {
                                            args,
                                            foreground: true,
                                            reload_after: true,
                                            label: item.label,
                                        },
                                    };
                                }
                                GenerateKind::WorkspaceTemplate { template_name } => {
                                    state.overlay = Overlay::GenerateMenu {
                                        items,
                                        selected,
                                        template_name_input: Some((
                                            template_name,
                                            TextInput::default(),
                                        )),
                                    };
                                }
                            }
                        }
                    }
                    _ => {
                        state.overlay = Overlay::GenerateMenu {
                            items,
                            selected,
                            template_name_input: None,
                        };
                    }
                }
            }
        }
    }

    state.dirty = true;
    true
}

/// Shared Unicode-safe text input key handler (cursor movement, delete word, clear line).
pub fn handle_text_input(input: &mut TextInput, key: KeyEvent) -> bool {
    if key.modifiers.contains(KeyModifiers::CONTROL) {
        return match key.code {
            KeyCode::Char('w') => {
                input.delete_word_back();
                true
            }
            KeyCode::Char('u') => {
                input.clear();
                true
            }
            KeyCode::Char('a') => {
                input.move_home();
                false
            }
            KeyCode::Char('e') => {
                input.move_end();
                false
            }
            _ => false,
        };
    }

    match key.code {
        KeyCode::Backspace => {
            input.backspace();
            true
        }
        KeyCode::Delete => {
            input.delete_char();
            true
        }
        KeyCode::Left => {
            input.move_left();
            false
        }
        KeyCode::Right => {
            input.move_right();
            false
        }
        KeyCode::Home => {
            input.move_home();
            false
        }
        KeyCode::End => {
            input.move_end();
            false
        }
        KeyCode::Char(c) => {
            input.insert_char(c);
            true
        }
        _ => false,
    }
}
