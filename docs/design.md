# lazymise Interface Direction

Follows the same dense, quiet operator-console visual language as `lazychezmoi`:

- **Canvas**: terminal reset color (transparent background)
- **Focus surface**: `#1f2430` + bold text
- **Primary text**: `#d8dee9`
- **Muted text**: `#737c8c`
- **Semantic accents**: cyan `#5ccfe6` (tasks/active), amber `#ffcc66` (warnings/pending), green `#bae67e` (success), red `#ff6171` (failure/destructive)
- **Layout**: 40/60 split on wide terminals (`>= 80` cols); single active pane on narrow terminals
- **Guardrails**:
  - Always show the exact `mise` command before executing high-impact actions (`upgrade`, `prune`, `new`).
  - Mask secret-like environment variables by default (`S` toggles reveal).
  - Never use color as the sole status indicator.
