# lazymise

<p align="center">
  <strong>English</strong> | <a href="README.zh-CN.md">简体中文</a>
</p>

A Vim-first, lazygit-inspired terminal workbench for [`mise`](https://mise.jdx.dev/) — uniting **Tasks**, **Tools**, **Env**, **Config & Health**, and **Command Log & Jobs** in one non-blocking TUI deeply aligned with `mise`'s official roadmap.

## Layout Overview

On wide terminals (`>= 96` columns), `lazymise` renders a 5-panel workbench with a left accordion sidebar (36%), a multi-subview detail pane (64%), and a toggleable bottom Command Log & Jobs drawer. Zoom any focused pane with `+` / `_` (`Normal` ↔ `Half` ↔ `Full`), or work seamlessly on narrow terminals (`< 96` columns) where the sidebar degrades to compact `1-5` header badges.

```text
┌─ lazymise │ ~/workspace/repo │ ENV:[default] │ [⠋ outdated] ─────── [+:zoom ?:help] ─┐
│┌─[1] TASKS (12 • 1 running)────┐┌─[Spec]  Output   Deps   History ─── [ ] switch ───┐│
││ ▾ [root] (4)                  ││ NAME     check                                    ││
││   ● check   [1.2s]            ││ TAGS     #ci    CACHE: ⚡ on                      ││
││   ✓ test    [0.8s • 14 runs]  ││ DEPENDS  tidy, test, lint                         ││
││   ✓ build   [2.1s]            ││ ───────────────────────────────────────────────── ││
││ ▸ exp (5)                     ││ SCRIPT / COMMAND (cached, zero render I/O)        ││
││ ▸ [global] (3)                ││ ...                                               ││
│└───────────────────────────────┘│                                                   ││
│┌─[2] TOOLS (8 • ↑2 • ⚠1 CVE)───┐│                                                   ││
││ ✓ rust 1.90.0  ↑ node 22→23   ││                                                   ││
│└───────────────────────────────┘│                                                   ││
│┌─[3] ENV (24 • 3 redacted)─────┐│                                                   ││
││ OUT_DIR, RUST_LOG, 🔒TOKEN... ││                                                   ││
│└───────────────────────────────┘│                                                   ││
│┌─[4] CONFIG & HEALTH (3 • 1W)──┐│                                                   ││
││ ✓ mise.toml  ⚠ doctor:1 warn  ││                                                   ││
│└───────────────────────────────┘└───────────────────────────────────────────────────┘│
│┌─[5] COMMAND LOG & JOBS (@:toggle) ─ 1 running ─────────────────────────────────────┐│
││ 13:20:04  ⠋ RUN  [USR] mise run check              [1.2s] (K:kill  Enter:inspect)  ││
││ 13:19:50  ✓ 0    [INT] mise outdated --json        [0.37s]                         ││
│└────────────────────────────────────────────────────────────────────────────────────┘│
│ Enter:run  A:args  f:force  w:watch │ x:menu  /:filter  E:env  ::palette  ?:help  q:quit │
└──────────────────────────────────────────────────────────────────────────────────────┘
```

## Five Panels & Sub-Views

| Panel | Key | Left List / Accordion Card | Right Pane Sub-Views (`[` / `]`) |
| --- | :---: | --- | --- |
| **Tasks** | `1` | Collapsible namespace / monorepo tree with run status, history duration, cache (`⚡`), and `#tag` badges | `Spec` · `Output` (live ANSI) · `Deps` · `History` |
| **Tools** | `2` | Installed & configured tools with 5-scope filter (`All`/`Active`/`Project`/`Missing`/`Prunable`), outdated arrows, and CVE badges | `Details` · `Outdated` · `Audit` (OSV.dev) · `Lock` |
| **Env** | `3` | Environment variables with 6-source filter (`All`/`Project`/`Local`/`Global`/`Tool`/`Redacted`) and secret lock badges | `Value & Source` · `PATH` · `Vars` |
| **Config & Health** | `4` | Active config hierarchy chain with `[TRUSTED]` / `[UNTRUSTED]` / `[OVR]` status and diagnostic counts | `File & Explain` · `Doctor` · `Settings` · `Daemons & Deps` |
| **Command Log & Jobs** | `5` | Live background jobs & transparent `mise` CLI audit log (toggle drawer with `@`) | Full ANSI job output & command details |

## Highlights

- **Zero-Blocking Async Loading**: Renders the first frame immediately without blocking on subprocesses; Tier-1 hot domains (`Tasks`, `Tools`, `Env`, `Config`, `Templates`) fan out in parallel while Tier-2 domains (`Registry`, `Outdated`, `Doctor`, `Audit`, `Lock`, `Settings`, `Daemons`, `Deps`) load in the background or on demand with generation-based stale-response cancellation and 0% CPU idle blocking.
- **`MISE_ENV` Profile Switching (`E`)**: Auto-discovers workspace `mise.<env>.toml` profiles (or accepts comma-separated profiles like `dev,local`), displays an interactive `ENV:[profile]` header badge, and injects `-E <profile>` consistently across all queries and commands.
- **Instant Registry Search & Install (`2` → `a`)**: Fuzzy-search `mise`'s 1,079+ tool registry in memory, pick a version from asynchronous `mise ls-remote`, choose project or global scope, and confirm the exact `mise use` command before running.
- **Supply-Chain Security & Lockfiles**: Run OSV.dev CVE audits (`mise audit --json`), upgrade vulnerable tools (`mise upgrade --vulnerable`), inspect `mise lock --dry-run` diffs, and update `mise.lock` with explicit confirmation.
- **Task Workflows & Monorepo Support**: Non-blocking background execution by default on `Enter` (with live top-right header status badges and streaming ANSI output), optional foreground interactive TTY handoff (`T`, with RAII terminal restoration and `$EDITOR` shell-word parsing), `#USAGE` (`usage_spec`) argument forms (`A`), `--force` runs (`f`), background `mise watch` (`w` with `watchexec` detection), `mise tasks validate` (`v`), hidden tasks (`H`), and monorepo `--all` discovery (`M`).
- **Environment Governance**: Combines `mise env --redacted --json` with heuristic secret detection; masks sensitive values by default, reveals with `S` (auto-re-masks after 30s), warns when copying secrets, and edits via `mise set` / `mise unset`.
- **Config Governance & Scaffolding**: Inline `mise config explain` diagnostics, structured `mise doctor` dashboard, searchable `mise settings`, experimental `[daemons]` & `[deps]` management, plus a unified `g` Generate & Scaffold modal covering all 7 official `mise generate` targets and custom `$MISE_TEMPLATES_DIR` / `~/.config/mise/templates` templates.
- **Multi-Job Process Management & ANSI Rendering**: Unique `JobId` isolation, Unix process-group termination (`SIGTERM` → `SIGKILL` with cleanup on exit and quit-confirmation guard), `\r` progress overwrite, and zero-dependency 16/256/TrueColor ANSI SGR rendering.
- **Safe Version-Level Uninstall vs. Config Removal (`d`)**: In `[2] TOOLS`, pressing `d` on an inactive, unreferenced, or prunable version runs `mise uninstall <tool>@<version>` without touching `mise.toml`; pressing `d` on an active configured tool runs `mise unuse <tool>` with an explicit config-path confirmation prompt.
- **Zero-Memory-Burden Discovery & Context-Aware 80/20 Footer**: Single-source-of-truth keymap powering a dynamic footer bar that adapts to the active panel, sub-view, and highlighted item (separating context actions and global shortcuts with `│` while preserving `x:menu ?:help q:quit` on narrow terminals), plus searchable/executable `?` Help, contextual `x` Action Menu, `:` / `Ctrl-p` Command Palette, and full mouse support (click panels/sub-tabs/profile badge, double-click primary actions, hover wheel scroll).
- **Accessible Terminal Aesthetics**: Transparent canvas, `NO_COLOR` / `CLICOLOR=0` / `TERM=dumb` and 256/16-color degradation, non-color redundant status glyphs, compact filtered tool `OPTIONS` metadata, and `unicode-width`-safe CJK/Emoji truncation.

## Installation & Quick Start

Requires Rust `1.88+` and `mise` installed on `PATH`.

```bash
# Install from source via mise
mise run install

# Or directly with Cargo
cargo install --path . --locked
```

### CLI Usage

```bash
lazymise                          # Launch in current directory on the Tasks panel
lazymise -C /path/to/repo         # Inspect a specific directory (long flag: --dir)
lazymise -E dev                   # Start with MISE_ENV=dev (long flag: --env)
lazymise -E dev,local -t tools    # Start with combined profile on the Tools panel
lazymise --tab config             # Start on Config & Health (legacy alias: --tab templates)
lazymise --tab jobs               # Start focused on Command Log & Jobs
```

| Flag | Values / Default | Description |
| --- | --- | --- |
| `-C`, `--dir <DIR>` | Current directory | Working directory to inspect |
| `-E`, `--env <ENV>` | None (`default`) | Initial `MISE_ENV` profile (e.g. `dev`, `prod`, or `dev,local`) |
| `-t`, `--tab <TAB>` | `tasks` \| `tools` \| `env` \| `config` \| `jobs` (default: `tasks`) | Initial panel to focus (`templates` accepted as an alias for `config`) |
| `-h`, `--help` | — | Print CLI help |
| `-V`, `--version` | — | Print version |

## Keybindings

All keybindings are defined in a single registry ([`src/app/keymap.rs`](src/app/keymap.rs)) that drives dispatch, footer hints, `?` Help, `x` Context Menu, and `:` Command Palette.

### Global

| Key | Action |
| --- | --- |
| `1` – `5` | Switch active panel (`Tasks` / `Tools` / `Env` / `Config & Health` / `Jobs & Log`) |
| `j` / `k` (or `↓` / `↑`) | Move cursor or scroll detail down / up |
| `gg` / `G` (or `Home` / `End`) | Jump to top / bottom (`g` opens Generate on Config; `Home` works everywhere) |
| `Ctrl-d` / `Ctrl-u` (or `PgDn` / `PgUp`) | Scroll half-page down / up |
| `h` / `l` (or `←` / `→`), `Tab` / `Shift-Tab` | Switch focus between Sidebar, Main detail, and Log drawer (`h`/`l` also fold/unfold Tasks groups) |
| `[` / `]` | Switch right pane sub-view |
| `+` / `_` (also `=` / `-`) | Zoom screen in / out (`Normal` ↔ `Half` ↔ `Full`) |
| `@` | Cycle Command Log & Jobs drawer (`Hidden` → `Compact` → `Expanded`) |
| `/` | Fuzzy filter current panel (`Enter` apply, `Esc` clear) |
| `Esc` | Clear active filter or close overlay |
| `x` | Open context action menu for current selection |
| `:` or `Ctrl-p` | Open global Command Palette |
| `E` | Switch active `MISE_ENV` (`-E`) profile |
| `R` | Reload `mise` state asynchronously |
| `r` | Re-run last executed task (or selected command in Jobs & Log) |
| `?` | Open searchable keybindings help (`Enter` executes highlighted action) |
| `q` / `Ctrl-c` | Quit (`q` prompts if jobs are running; double `Ctrl-c` within 3s force-quits) |

### Panel `[1]` Tasks

| Key | Action |
| --- | --- |
| `Enter` | Run task in background with live ANSI output & top-right status (or fold group; auto-opens args form if required) |
| `T` | Run task in foreground interactive TTY |
| `A` | Run task with arguments / `#USAGE` flags form |
| `f` | Run task with `--force` (bypass `sources`/`outputs` cache) |
| `w` | Watch task via `mise watch -t <task>` |
| `Space` or `za` | Fold / unfold task namespace group |
| `H` | Toggle hidden tasks (`--hidden`) |
| `M` | Toggle monorepo all subpackage tasks (`--all`) |
| `v` | Validate tasks (`mise tasks validate`) |
| `e` | Edit task source file in `$EDITOR` (fallback `$VISUAL`, default `nvim`) |
| `y` | Copy `mise run <task>` command to clipboard |
| `F` | Toggle auto-follow tailing in `Output` view |

### Panel `[2]` Tools

| Key | Action |
| --- | --- |
| `a` | Search `mise` registry and add tool (`mise use`) |
| `i` | Install configured tools (`mise install`) |
| `u` | Upgrade selected tool (`mise upgrade <tool>`) |
| `U` | Upgrade all outdated tools (`mise upgrade`) |
| `V` | Upgrade vulnerable tools (`mise upgrade --vulnerable`) |
| `d` | Uninstall selected inactive/old version (`mise uninstall <tool>@<ver>`) or remove active tool from config (`mise unuse <tool>`) |
| `p` | Prune unused tool versions (`mise prune`) |
| `L` | Preview `mise lock --dry-run` (apply `mise lock` via `x` menu) |
| `A` | Run OSV.dev security audit (`mise audit --json`) |
| `O` | Refresh outdated tools check (`mise outdated --json`) |
| `f` | Cycle tool scope (`All` / `Active` / `Project` / `Missing` / `Prunable`) |
| `o` | Open tool homepage or release URL in browser |
| `e` | Edit source `mise.toml` for selected tool |

### Panel `[3]` Env

| Key | Action |
| --- | --- |
| `a` | Add new environment variable (`mise set`) |
| `e` | Edit selected environment variable (`mise set`) |
| `d` | Unset selected environment variable (`mise unset`) |
| `S` | Toggle reveal secret value (auto-re-masks in 30s) |
| `y` | Copy environment variable value to clipboard |
| `Y` | Copy `KEY=VALUE` pair to clipboard |
| `f` | Cycle env source filter (`All` / `Project` / `Local` / `Global` / `Tool` / `Redacted`) |

### Panel `[4]` Config & Health

| Key | Action |
| --- | --- |
| `Enter` / `e` | Edit selected config file in `$EDITOR` |
| `t` | Trust / untrust selected config (`mise trust` / `--untrust`) |
| `F` | Format `mise.toml` (`mise fmt`) |
| `D` | Refresh `mise doctor --json` diagnostics |
| `g` | Open Generate & Scaffold menu (`mise generate` + custom templates) |
| `s` | Start / stop selected daemon (in `Daemons & Deps` sub-view) |
| `S` | Sync project dependencies (`mise deps`) |

*(Additional doctor & service actions such as `mise reshim`, `mise doctor project`, and daemon restart are available in the `x` Context Menu.)*

### Panel `[5]` Jobs & Command Log

| Key | Action |
| --- | --- |
| `Enter` | Inspect selected job output in main pane |
| `K` | Kill running background job process group |
| `r` | Re-run selected command in background |
| `y` | Copy exact CLI command to clipboard |
| `c` | Clear completed entries from Command Log |
| `i` | Toggle showing internal read/refresh `mise` commands |
| `F` | Toggle auto-follow tailing in job output |

## Development

All repeatable workflows are exposed via [`mise.toml`](mise.toml):

```bash
mise tasks        # List all project tasks
mise run dev      # Run lazymise in debug mode (cargo run)
mise run check    # Run workspace hygiene (including EN/ZH README sync check) + tests + clippy (-D warnings)
mise run install  # Build release binary and install to ~/.cargo/bin
```

For interface conventions, color degradation, layout rules, guardrails, and concurrency architecture, see [`docs/design.md`](docs/design.md).
