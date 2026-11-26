# Agent Instructions — lazymise

- Read `README.md` and `mise.toml` before adding files or scripts.
- Expose repeatable workflows via `mise.toml` (`[tasks]`) or `.mise/tasks/` rather than root-level shell scripts.
- Keep build artifacts and logs under `outputs/`.
- Run `mise run check` before finishing a task.
- Keep `README.md` and `README.zh-CN.md` synchronized whenever documentation changes (enforced by `mise run check`).
- Run `mise run install` after code changes pass checks so the global `lazymise` binary stays up to date.
