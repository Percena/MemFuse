#!/usr/bin/env bash
set -uo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
ENV_FILE="$SCRIPT_DIR/.env"

# ── Load .env (safe parser: only KEY=VALUE lines, skip comments/blanks) ──
if [[ -f "$ENV_FILE" ]]; then
  while IFS= read -r rawline || [[ -n "$rawline" ]]; do
    # Skip lines starting with # (comments)
    [[ "$rawline" =~ ^[[:space:]]*# ]] && continue
    # Skip empty or whitespace-only lines
    trimmed="${rawline//[[:space:]]/}"
    [[ -z "$trimmed" ]] && continue
    # Strip inline comments (# outside quotes)
    rawline="${rawline%%#*}"
    # Trim trailing whitespace
    rawline="${rawline%%[[:space:]]}"
    # Validate KEY=VALUE format
    if [[ "$rawline" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; then
      export "$rawline"
    fi
  done < "$ENV_FILE"
else
  echo "⚠  No .env file found at $ENV_FILE — using defaults only."
fi

# ── Defaults (only set if not already defined) ──────────────────
: "${MEMFUSE_WORKSPACE_ROOT:=$HOME/.memfuse/data}"
: "${MEMFUSE_SOURCE_KIND:=managed}"
: "${MEMFUSE_BIND_ADDR:=127.0.0.1:8720}"
: "${MEMFUSE_ACCOUNT_ID:=default}"
: "${MEMFUSE_USER_ID:=default}"
: "${MEMFUSE_AGENT_ID:=default}"
: "${MEMFUSE_MAX_RETRIES:=3}"
: "${MEMFUSE_RETRY_BASE_DELAY_MS:=500}"
: "${MEMFUSE_RETRY_MAX_DELAY_MS:=8000}"
: "${MEMFUSE_CB_FAILURE_THRESHOLD:=5}"
: "${MEMFUSE_CB_RESET_TIMEOUT_MS:=300000}"
: "${MEMFUSE_SERVER_URL:=http://127.0.0.1:8720}"

# ── Expand leading ~ in path env vars ───────────────────────────
# Neither this .env parser nor the Rust runtime performs tilde
# expansion, so "~" would be treated as a literal relative path.
# MEMFUSE_WORKSPACE_ROOT always has a default (set above).
MEMFUSE_WORKSPACE_ROOT="${MEMFUSE_WORKSPACE_ROOT/#\~/$HOME}"
# MEMFUSE_SOURCE_PATH is optional (managed mode doesn't need it).
[[ -n "${MEMFUSE_SOURCE_PATH+set}" ]] && MEMFUSE_SOURCE_PATH="${MEMFUSE_SOURCE_PATH/#\~/$HOME}"

# ── Ensure workspace root exists ────────────────────────────────
mkdir -p "$MEMFUSE_WORKSPACE_ROOT"

# ── Launch server (env vars inherited by exec) ───────────────────
exec cargo run -p mfs-server
