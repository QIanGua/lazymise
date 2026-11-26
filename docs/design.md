# lazymise 2.0 Interface & Architecture Design

`lazymise` follows a dense, quiet operator-console visual language inspired by `lazygit` and `lazychezmoi`, paired with a zero-blocking, event-driven architecture aligned with [`mise`](https://mise.jdx.dev/)'s core pillars.

---

## 1. Visual Language, Color Palette & Degradation

Implementation: [`src/ui/theme.rs`](../src/ui/theme.rs), [`src/ui/widgets.rs`](../src/ui/widgets.rs)

- **Canvas**: Terminal reset color (transparent background) so `lazymise` blends naturally with the user's terminal theme.
- **Borders**: Rounded box borders (`BorderType::Rounded`); focused borders highlight in `CYAN` (or `BOLD` in `NO_COLOR`), unfocused borders recede to `SUBTLE` (`DIM`).
- **Wide-Character Safety**: All truncation, alignment, and text-input cursor calculations use `unicode-width` (`truncate_display`, `shorten_path`) so CJK characters and Emoji never corrupt cell layout.
- **Notifications**: Graded floating Toast cards (`Info`, `Success`, `Warning`, `Error`) render at the top-right (`y + 2`) without obscuring the header, primary list, or sub-view tabs; on ultra-short terminals (`< 12` rows), active toasts fall back inline into the footer.

### Semantic Color Palette & Terminal Depth Mapping

| Token | Role | TrueColor (`RGB`) | ANSI 256 (`Indexed`) | ANSI 16 | `NO_COLOR` |
| --- | --- | --- | --- | --- | --- |
| `SURFACE` | Focused row / selection surface | `#1f2430` | `235` | `DarkGray` | None (`REVERSED\|BOLD` focused, `UNDERLINED` unfocused) |
| `SURFACE_ELEVATED` | Brand pill / active filter pill | `#262c3a` | `236` | `DarkGray` | None (`REVERSED\|BOLD`) |
| `PRIMARY` | Primary text / section headers | `#d8dee9` | `253` | `White` | None |
| `MUTED` | Secondary labels / metadata | `#737c8c` | `243` | `Gray` | `DIM` |
| `SUBTLE` | Unfocused borders / separators | `#4c566a` | `239` | `DarkGray` | `DIM` |
| `CYAN` | Focus border / active accents / key labels | `#5ccfe6` | `80` | `Cyan` | `BOLD` (where emphasized) |
| `AMBER` | Warnings / pending / fuzzy match / `MISE_ENV` | `#ffcc66` | `221` | `Yellow` | `BOLD\|UNDERLINED` (matches) |
| `GREEN` | Success / running job indicator | `#bae67e` | `150` | `Green` | None |
| `RED` | Failure / destructive / CVE alerts | `#ff6171` | `203` | `Red` | None |
| `PURPLE` | Secondary semantic highlights | `#c3a6ff` | `147` | `Magenta` | None |

Capability detection (`detect_color_capability_from`) checks environment variables in strict priority order:
1. `NO_COLOR` (non-empty, per [no-color.org](https://no-color.org/)), `CLICOLOR=0`, or `TERM=dumb` → `NoColor`
2. `COLORTERM` containing `truecolor` / `24bit`, or modern `TERM` (`direct`, `kitty`, `wezterm`, `alacritty`, `ghostty`, `iterm`) → `TrueColor`
3. `TERM` containing `256color`, `tmux`, or `screen` → `Ansi256`
4. Other non-empty `TERM` → `Ansi16` (empty/unset defaults to `TrueColor`)

### Non-Color Redundant Status Glyphs

Color is **never** the sole indicator of state. Every status carries an explicit Unicode glyph (`theme::glyphs`):

| Glyph | Constant | Semantic Meaning |
| :---: | --- | --- |
| `⠋`–`⠏` | `SPINNER` | Active async fetch or running background job |
| `✓` / `✗` | `OK` / `FAIL` | Succeeded / failed task, tool, config trust, or command |
| `●` / `○` | `RUNNING` / `IDLE` | Active running job / idle item |
| `↑` | `UPGRADE` | Outdated tool with newer upstream version |
| `⚠` | `WARN` | Doctor warning, CVE finding, or `config explain` diagnostic |
| `✂` | `PRUNE` | Prunable unused tool version |
| `🔒` / `🔓` | `LOCK` / `UNLOCK` | Masked secret / temporarily revealed secret |
| `⚙` / `◈` | `TOOL` / `VAR` | Tool-injected env var / template variable |
| `⚡` | `CACHE` | Task with `sources`/`outputs` caching enabled |
| `▾` / `▸` | `FOLD_OPEN` / `FOLD_CLOSED` | Expanded / collapsed task namespace group |
| `•` / `·` | `BULLET` / `INACTIVE` | Active separator / inactive item |

---

## 2. Spatial Layout, Accordion, Zoom & Responsive Degradation

Implementation: [`src/ui/layout.rs`](../src/ui/layout.rs), [`src/ui/mod.rs`](../src/ui/mod.rs), [`src/app/mouse.rs`](../src/app/mouse.rs)

### Five-Panel Accordion Workbench (Wide Terminals `>= 96` Columns)

- **Top Header (1 row)**: Brand pill (`lazymise`), shortened working directory, clickable `ENV:[profile]` badge, async loading spinner (`[⠋ domain]`), running jobs count, active zoom badge, and `[+:zoom ?:help]`.
- **Left Accordion Sidebar (36% width in `Normal` mode)**: Stacks the 4 primary panels (`[1] TASKS`, `[2] TOOLS`, `[3] ENV`, `[4] CONFIG & HEALTH`).
  - The focused sidebar panel expands with `Constraint::Fill(1)` (when `[5] JOBS` is focused, `[1] TASKS` remains expanded as the reference panel).
  - The 3 inactive panels collapse into **3-row live summary cards** displaying counts, running state, outdated/CVE alerts, and doctor diagnostics.
  - When `terminal_height < 26` (`COMPACT_ACCORDION_HEIGHT`) or `sidebar.height < 16`, collapsed cards compress to **1-line title strips**; below 6 sidebar rows they drop to `0` so the active list is never crushed.
- **Right Multi-Subview Main Pane (64% width in `Normal` mode)**: Displays context-specific sub-views switched via `[` / `]` or mouse click:
  - Tasks: `Spec` · `Output` · `Deps` · `History`
  - Tools: `Details` · `Outdated` · `Audit` · `Lock`
  - Env: `Value & Source` · `PATH` · `Vars`
  - Config & Health: `File & Explain` · `Doctor` · `Settings` · `Daemons & Deps`
  - Jobs: Full ANSI output & command details
- **Bottom Command Log & Jobs Drawer (`[5]`)**: Cycled via `@` across `Hidden` (0 rows), `Compact` (4 rows, default), and `Expanded` (up to 10 rows / 50% height, reserving at least 5 body rows).
- **Bottom Footer (1 row)**: Dynamically populated from `keymap::footer_hints_for_state` for the effective active panel.

### Screen Zoom Modes (`+` / `_`)

| Mode | Sidebar / Main Split | Command Log Drawer |
| --- | --- | --- |
| `Normal` | `36% / 64%` | Respects `@` (`Hidden` / 4 rows / up to 10 rows) |
| `Half` | `58% / 42%` (Sidebar focused) or `28% / 72%` (Main focused) | Expands up to 10 rows when `Log` is focused |
| `Full` | `100%` for the focused region (`Sidebar`, `Main`, or `Log`) | Hidden unless `Log` is focused (`100%` body) |

### Narrow-Screen Degradation (`< 96` Columns)

Below `NARROW_WIDTH_THRESHOLD` (96 columns):
- The left accordion is replaced by compact clickable header badges (`[1:TASKS] ... [5:JOBS]`, or `[1]..[5]` under 66 columns) plus an explicit focus indicator (`[LIST ←→ detail]`, `[list ←→ DETAIL]`, or `[LOG]`).
- The body renders a single focused pane (`Sidebar`, `Main`, or `Log`), toggled smoothly with `h` / `l` or `Tab`.
- Below `24x6`, a single warning line asks the user to resize cleanly without panicking.

### `LayoutMap` & Full Mouse Support

During `ui::render`, every interactive region registers its exact `Rect` into `LayoutMap` (`HeaderProfile`, `HeaderPanel`, `SidebarPanel` outer + inner list excluding filter/scope headers, `MainSubTab`, `MainBody`, `LogDrawer`, `Overlay`):
- **Single click**: Focuses panel, selects row, switches sub-view tab, or opens Profile Switcher; clicking outside a modal overlay dismisses it.
- **Double click (400ms window)**: Triggers the panel's primary action (Tasks: run foreground; Tools: upgrade; Env: toggle reveal; Config: edit; Jobs: inspect).
- **Hover wheel scroll**: Scrolls the region directly under the cursor without stealing keyboard focus.

---

## 3. Single-Source-of-Truth Discoverability & Safety Guardrails

Implementation: [`src/app/keymap.rs`](../src/app/keymap.rs), [`src/app/reducer.rs`](../src/app/reducer.rs), [`src/app/overlay.rs`](../src/app/overlay.rs), [`src/model/env.rs`](../src/model/env.rs), [`src/runner/`](../src/runner/)

### Unified Keymap & Overlays

- `keymap::all_bindings()` is the single source of truth for key resolution, contextual footer hints, searchable/executable `?` Help, contextual `x` Action Menu, and `:` / `Ctrl-p` Command Palette (with weighted fuzzy ranking across label, command preview, and description). Unit tests assert zero key conflicts within any panel scope.
- Ten modal overlays (`Help`, `Filter`, `ContextMenu`, `CommandPalette`, `ProfileSwitcher`, `Confirm`, `TaskArgsForm`, `RegistryPicker`, `EnvSetForm`, `GenerateMenu`) share a Unicode-safe `TextInput` supporting cursor navigation, `Ctrl-w` word delete, and `Ctrl-u` clear.

### Command Preview & Confirmation Guardrails

Every mutating or high-impact operation displays an explicit `Confirm` modal showing the **exact `mise` command** (including `-E <profile>` when active) and an impact summary before execution:
- Tools: `mise use <tool>@<version>`, `mise install`, `mise upgrade [tool|--vulnerable]`, `mise unuse <tool>`, `mise prune`, `mise lock`
- Env: `mise set KEY=VAL` (with target file selection: `mise.toml` / `mise.local.toml` / `mise.<env>.toml`), `mise unset KEY`
- Config & Services: `mise trust [--untrust] <path>`, `mise fmt`, `mise reshim`, `mise daemons start|stop|restart <name>`, `mise deps`, `mise generate <target>`, and custom template scaffolding

### Secret Redaction & Clipboard Safety

- `EnvViewModel` merges `mise env --redacted --json` with 29 heuristic patterns (`SECRET`, `TOKEN`, `PASSWORD`, `API_KEY`, `ACCESS_KEY`, `PRIVATE_KEY`, `JWT`, `BEARER`, `DATABASE_URL`, `DSN`, `COOKIE`, `PRIVATE`, `CERT`, `SIGNING`, `SSH_KEY`, etc.).
- Sensitive values are masked by default; pressing `S` reveals the selected key with a **30-second auto-re-mask timer** (`SECRET_REVEAL_TTL`).
- Copying a sensitive value (`y` / `Y`) raises a visible `Warning` toast. Clipboard delivery tries native backends (`pbcopy`, `wl-copy`, `xclip`, `xsel`) and falls back to OSC 52 (with tmux DCS passthrough).

### Process Isolation & Terminal Integrity

- **Background jobs (`src/runner/jobs.rs`)**: Tracked by monotonic `JobId(u64)` so concurrent runs never interleave. On Unix, each job starts in its own process group (`process_group(0)`); killing (`K`) sends `SIGTERM` to `-pgid`, escalating to `SIGKILL` after a 400ms grace period. `BackgroundRunner::drop` reaps all remaining jobs automatically.
- **Quit guard**: Pressing `q` with active background jobs opens a confirmation modal; double `Ctrl-c` within 3 seconds (`FORCE_QUIT_WINDOW`) terminates all jobs and exits cleanly.
- **Foreground TTY handoff (`src/runner/foreground.rs`, `src/main.rs`)**: Before suspending the TUI for interactive tasks, `mise watch`, or `$EDITOR`, the crossterm input thread is paused via `Mutex + Condvar` (eliminating stdin races), `$EDITOR` is parsed via POSIX `split_shell_words` (supporting `EDITOR="code --wait"`), and a `TerminalGuard` RAII + panic hook guarantee raw mode, mouse capture, and alternate screen are always restored.
- **ANSI & Byte-Safe Output (`src/runner/ansi.rs`)**: Reads raw bytes with UTF-8 lossy decoding, supports `\r` progress-bar line overwrite, caps each job buffer at 4,000 lines, and renders 16/256/TrueColor ANSI SGR sequences with zero external dependencies.

---

## 4. Async Tiered Loading & Event-Driven Architecture

Implementation: [`src/main.rs`](../src/main.rs), [`src/events.rs`](../src/events.rs), [`src/mise/mod.rs`](../src/mise/mod.rs), [`src/mise/loader.rs`](../src/mise/loader.rs)

```text
Crossterm Input Thread (Key / Mouse / Resize) ──┐
MiseLoader (Tier-1 Hot / Tier-2 Lazy threads) ──┼──► mpsc::Receiver<AppEvent>
BackgroundRunner (Job Started/Output/Exit)    ──┘              │
                                                               ▼
                                               App::handle_event(ev) [State Machine]
                                                               │
                                                        dirty == true ?
                                                               ▼
                                     terminal.draw(|f| ui::render(f, &mut state, &mut layout_map))
```

- **No Heavy Async Runtime**: Built solely on `std::thread`, `std::sync::mpsc`, and `AtomicU64` generation counters — fast compilation, minimal binary size, and zero async-runtime overhead.
- **Zero-Blocking Startup**: `App::new` spawns Tier-1 hot-path fetches immediately and returns so the first frame renders without waiting for `mise` CLI calls.
  - **Tier-1 Hot Path** (`HOT_TIMEOUT = 5s`): `Tasks`, `Tools`, `Env` (concurrent `--json-extended`, `--redacted --json`, `--paths`, `vars --json`), `Config` (`config ls --json` + `trust --show`), and filesystem `Templates` discovery.
  - **Tier-2 Background Warmup & On-Demand Lazy Fetches** (`SLOW_TIMEOUT = 20s`): Background warmup for `Registry`, `Outdated`, and `Doctor`; on-demand fetches for `Audit`, `LockDryRun`, `TaskInfo`, `TaskDeps`, `TaskHistory`, `ToolDetail`, `ToolVersions` (`ls-remote`), `ConfigExplain`, `Settings`, `Daemons`, and `Deps`.
- **Generation Cancellation & Subprocess Timeouts**: Every full reload or `MISE_ENV` switch increments `generation: u64`; stale worker responses are discarded before mutating state. Every subprocess runs in its own Unix process group with concurrent stdout/stderr draining and strict timeout termination.
- **Non-Zero JSON Resilience**: `MiseClient::run_json_allowing_nonzero` parses valid JSON from `mise doctor --json` and `mise audit --json` even when findings cause exit code `1`.
- **Zero-I/O Render & 0% Idle CPU**: File scripts and config previews are preloaded into `ScriptCache` during selection/reducer transitions — `ui::render` performs zero disk or subprocess I/O. The main loop uses `rx.recv()` (0% CPU) when idle and switches to a 100ms tick only while spinners, toasts, or secret-reveal timers are active.
- **Transparent Command Log**: A 200-entry ring buffer (`CommandLog`) records both `User` and `Internal` `mise` invocations with timestamps, durations, exit codes, and summaries so users always see and can learn/copy the exact CLI commands behind every UI action.

---

## 5. Alignment with Official `mise` Roadmap

1. **Three Core Pillars + Config Governance**: Elevates `Tasks`, `Tools`, and `Env` as first-class panels while replacing v1's custom `Templates` tab with **Config & Health** (`--tab templates` retained as a backward-compatible alias).
2. **`MISE_ENV` Profiles Everywhere**: Profile discovery scans `mise.<env>.toml`, `.mise.<env>.toml`, `.mise/config.<env>.toml`, and `mise/config.<env>.toml` (excluding non-profile files like `mise.local.toml` and `mise.lock`), propagating `-E <profile>` uniformly across reads, mutations, and foreground/background runs.
3. **Modern Non-Deprecated CLI Surface**: Uses `mise ls --no-global` (never deprecated `--local`), `mise env --json-extended`, `mise registry --json`, `mise audit --json`, `mise lock --dry-run`, `mise config explain --json`, `mise doctor --json`, `mise tasks validate`, `mise tasks ls --all` for monorepos, and `usage_spec` argument parsing.
4. **Unified Official Generation & Custom Templates**: Consolidates all 7 official `mise generate` targets (`config`, `devcontainer`, `github-action`, `git-pre-commit`, `task-docs`, `task-stubs`, `tool-stub`) alongside optional custom workspace templates (`$MISE_TEMPLATES_DIR` / `~/.config/mise/templates`) in one `g` modal.
5. **Graceful Experimental Feature Support**: Queries `mise daemons --json` and `mise deps --list` on demand in `Config & Health → Daemons & Deps`, degrading silently to clear guidance when unconfigured.
