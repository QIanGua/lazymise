# lazymise

A Vim-first terminal workbench for `mise` (Tasks, Tools, Env, and Workspace Templates).

## Quick Start

```bash
lazymise          # Launch TUI in current directory (alias: lm)
lazymise --dir .  # Launch TUI targeting a specific directory
```

## Keybindings

- `1` / `2` / `3` / `4`: Switch between **Tasks**, **Tools**, **Env**, **Templates**
- `j` / `k`, `g` / `G`, `Ctrl-d` / `Ctrl-u`: Move cursor / scroll preview
- `h` / `l` or `Tab`: Switch focus between list and preview
- `/`: Incremental filter (`Enter` apply, `Esc` clear)
- `za` or `Space`: Fold / unfold namespace group
- `Enter`: Run selected task in foreground TTY (suspends TUI)
- `x`: Run selected task in background with live streaming output
- `r`: Re-run last executed task
- `w`: Run `mise watch -t <task>` in foreground
- `e`: Open task source file in `$EDITOR` (default `nvim`)
- `y`: Copy command or env value to clipboard
- `R`: Reload `mise` state
- `?` / `q`: Toggle help / quit

## Development

```bash
mise tasks       # List all project tasks
mise run dev     # Run in debug mode
mise run check   # Run tidy + tests + clippy
mise run install # Install release binary
```
