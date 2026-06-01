# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

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
