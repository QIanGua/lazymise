# lazymise Strategy

## Vision

<!-- vision: author-approved 2026-11-26 -->

`lazymise` exists so that developers can inspect, steer, and debug a `mise` workspace interactively without memorizing subcommands or losing terminal flow.
It serves terminal-native engineers who manage tools, tasks, environment variables, and configuration with `mise`, and it turns `mise`'s CLI surface into a responsive, keyboard-first operator workbench.
It owns exactly one thing: the interactive terminal control plane over the installed `mise` CLI.

### Mise is the source of truth

`lazymise` delegates every domain decision, resolution rule, and side effect to the installed `mise` binary rather than re-implementing `mise` internals.
Its top-level mental model mirrors `mise`'s official pillars - tasks, tools, environment, and configuration health - instead of personal conventions or bespoke wrapper abstractions.
When `mise` evolves its CLI contracts or deprecates flags, `lazymise` adapts to the non-deprecated surface and degrades gracefully when optional or experimental features are unconfigured.
File editing belongs to the user's `$EDITOR`, not to an embedded text editor inside `lazymise`.

### Every action is visible and safe by default

No mutating or destructive `mise` command executes without first showing the exact CLI invocation and its scope for explicit confirmation.
Every command `lazymise` runs is recorded in a transparent command log so the interface teaches the underlying `mise` CLI rather than hiding it behind magic.
Secret-like and `mise`-redacted environment values stay masked by default, revert to masked automatically after temporary inspection, and never leak in plaintext into command previews or logs.
Terminal state and child processes are treated as invariants: foreground handoffs always restore the terminal on failure or panic, and background jobs are isolated in process groups and reaped cleanly on exit.

### Dense, discoverable, keyboard-first ergonomics

The interface follows a dense, quiet operator-console aesthetic with a transparent canvas that blends into the user's terminal theme.
Every command is reachable from Vim-first keybindings driven by a single conflict-checked keymap registry that also generates footer hints, searchable help, contextual menus, and the command palette.
Spatial layout adapts across wide, compact, and narrow terminals with zoom modes and direct mouse interaction without requiring users to memorize hidden shortcuts.
Color is never the sole status indicator; every state carries an explicit glyph, respects `NO_COLOR` and reduced color depths, and handles CJK and Emoji display widths without breaking layout.

### Instant startup and lean runtime discipline

The UI renders its first frame immediately without blocking on subprocesses, loads workspace data asynchronously in priority tiers with generation-based cancellation and timeouts, and consumes zero CPU while idle.
Rendering stays strictly read-only with zero disk or subprocess I/O on the draw path.
Dependencies remain minimal and compile fast as a single native binary; small, focused utilities are preferred over pulling in heavy async runtimes or sprawling frameworks.

### Scope

`lazymise` is not a replacement for `mise`, not a standalone package manager or task runner, not a TOML configuration editor, not a terminal multiplexer, and not a GUI desktop application.
Personal templates, team-specific generators, and local workflows remain outside `lazymise`'s core concepts and surface only through official `mise` integration points or optional user directories.
The repository holds itself to its own standard by driving all repeatable hygiene, test, and lint workflows through `mise.toml` with zero compiler or clippy warnings.

A change aligns when it makes an official `mise` workflow faster to inspect, safer to execute, or easier to discover in the terminal while keeping `mise` authoritative and the UI non-blocking.
A change should be resisted when it duplicates `mise` logic locally, hides the executed CLI command, weakens secret or process safety, adds heavy runtime dependencies for minor convenience, or promotes personal conventions into top-level workspace concepts.
