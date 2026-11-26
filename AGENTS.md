# Agent Instructions — lazymise

- Read `README.md` and `mise.toml` before adding files or scripts.
- Expose repeatable workflows via `mise.toml` (`[tasks]`) or `.mise/tasks/` rather than root-level shell scripts.
- Keep build artifacts and logs under `outputs/`.
- Run `mise run check` before finishing a task.
