#!/usr/bin/env bash
# Tools from .mise.toml, and a codegraph index for this checkout: a worktree under .claude/worktrees would otherwise
# be answered from the main checkout's index, which holds another branch.
set -euo pipefail
cd "$CLAUDE_PROJECT_DIR"

if [[ "${CLAUDE_CODE_REMOTE:-}" == true ]] && ! command -v mise >/dev/null; then
  curl -fsSL https://mise.run | sh >/dev/null
  export PATH="$HOME/.local/bin:$PATH"
  mise trust --quiet
fi

command -v mise >/dev/null || exit 0
mise install --quiet >&2
[[ -d .codegraph ]] || mise exec -- codegraph init --yes >/dev/null
