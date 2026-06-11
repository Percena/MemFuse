# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-06-11

### Fixed

- **Claude Code hooks now actually fire**: the installer previously wrote a
  malformed `hooks: [{event, cmd}]` array into `.claude/settings.local.json`;
  it now writes the canonical object schema
  (`hooks: { EventName: [{ matcher?, hooks: [{ type, command, timeout }] }] }`)
  and merges without clobbering user-defined hooks (see
  `docs/review/2026-06-10-comprehensive-review.md`, P0-1)
- SDK `bin/` entry wrappers (`memfuse`, `memfuse-mcp`, `memfuse-setup`, hook
  scripts) are now committed — fresh clones produce a working npm package (P0-2)
- Unified the default server port to **18720** everywhere with a single source
  of truth per language (Rust `mfs-types`, TypeScript `sdk/src/shared/config.ts`);
  override via `.env` (`MEMFUSE_BIND_ADDR` / `MEMFUSE_SERVER_URL` /
  `MEMFUSE_PORT` for docker compose) (P0-3)
- `callBackend` no longer fabricates a success payload when the (non-canvas)
  backend is unreachable — network errors propagate so hooks/MCP/CLI report
  degradation instead of silently dropping writes (P0-4)
- Stop hook reads Claude Code's `assistant_message` field (was reading a
  nonexistent `last_assistant_message`) (P1-1)
- PreToolUse memory hints are emitted as JSON `hookSpecificOutput.additionalContext`
  on Claude Code (plain stdout is not injected into model context for
  PreToolUse); deliberately no `permissionDecision` (P1-2)
- Hook commands pass `--platform=<p>` explicitly; payload-shape platform
  detection is now only a fallback (Claude Code Bash calls were misdetected
  as Codex) (P1-3)

### Added

- MCP tools `resolve_context` / `inject_context` / `store_observation` /
  `commit_session` accept an optional `session_id` so agent-initiated memory
  operations share the session used by lifecycle hooks (P1-4)
- `POST /context/resolve` accepts `recall_source`; passive hook injections
  send `"auto"` and skip recall reinforcement, so the forgetting curve only
  strengthens on explicit retrieval / `cite_memories` (P1-5)
- Codex installer prints a capability note: lifecycle hooks require a Codex
  build with hooks support; otherwise MemFuse runs in MCP + Skill mode (P1-6)
- `Cargo.lock` is now committed for reproducible binary builds

### Removed

- Stray scratch file `crates/mfs-retrieval/tests_planner_scratch.rs`

## [0.1.0] - 2026-05-26

### Added

- Core memory server with embedded SQLite (single binary, zero external dependencies)
- RESTful API with 117 endpoints and OpenAPI/Swagger documentation
- Session lifecycle: create, observe, commit, archive
- Episodic memory with Ebbinghaus forgetting curve and spacing-effect reinforcement
- Fact system with subject-predicate-value triples, temporal validity, and confidence tracking
- Behavioral heuristic learning (draft -> candidate -> confirmed lifecycle)
- Multi-strategy search: precision, diverse, recent, comprehensive
- Tiered semantic index: high-level (L0/L1) and detail (L2+) stored in separate table pairs for future cloud/local physical split
- Hierarchical indexing: L0 abstract, L1 overview, L2 full content
- Code repository import via localfs, git, and URL connectors
- AST skeleton extraction for Rust, Python, TypeScript, JavaScript, Go, Java, C/C++
- Active Overlay state machine with conflict detection (mfs-planning crate)
- Deterministic fallback mode (45 regex rules, keyword search, Jaccard matching) when no LLM keys are configured
- Rust CLI (mfs-cli) with 58 offline/diagnostic commands for direct workspace access
- TypeScript SDK with 110 CLI commands and platform adapters
- MCP server with 43 tools for agent-agnostic memory operations
- Claude Code integration: 8 lifecycle hooks + Skill + MCP server
- Codex integration: 3 lifecycle hooks + Skill + MCP server
- Systemd and macOS LaunchAgent service installation
- Docker and docker-compose support
- Privacy filtering and secret sanitization in observation capture
- SSRF protection for URL resource imports
- Rate limiting, CORS, and API key authentication
- Auto-consolidation ("dream") loop for cross-session memory compaction
- Pluggable embedding providers (Jina, OpenAI-compatible)
- Pluggable LLM providers (OpenAI-compatible) with circuit breaker resilience
