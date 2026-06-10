# MemFuse — Memory HUB for AI Agents

[![Rust](https://img.shields.io/badge/Rust-1.85+-000000)](https://www.rust-lang.org/)
[![TypeScript](https://img.shields.io/badge/TypeScript-5.8+-3178C6)](https://www.typescriptlang.org/)
[![License](https://img.shields.io/badge/License-MIT-green)](LICENSE)

**MemFuse** is a persistent memory hub that any AI agent or application can plug into. It runs as a local service, remembers what matters across sessions, and grows into a personalized mental model of how you think and work.

> **Not another RAG pipeline.** Most memory frameworks store every detail and re-retrieve it verbatim. MemFuse works like human memory — it knows *what* exists and *where* to find it, surfaces the right signal at the right time, and lets unimportant details fade naturally. The goal is guidance, not a database dump.

## Why MemFuse

| Traditional RAG Memory | MemFuse |
|------------------------|---------|
| Store all content, retrieve by similarity | Store facts + signals, guide agents to the source |
| Flat vector store, uniform treatment | Tiered: L0 abstracts / L1 overviews / L2 full content |
| No decay — everything stays equally "relevant" | Ebbinghaus forgetting curve with reinforcement on recall |
| Passive: only answers when asked | Proactive: injects context at session start, per prompt, before file reads |
| Coupled to one agent | Universal hub: any app via REST API, any agent via MCP/hooks |
| Static snapshots | Living memory: facts evolve, episodes consolidate, heuristics learn |

**Long-term vision**: MemFuse aligns to your personal mental model. The more you use it, the closer it gets to how you think — your preferences, your patterns, your blind spots, your decision style.

---

## Quick Start

### 1. Install and Start the Server

Download the latest release from [GitHub Releases](https://github.com/Percena/MemFuse/releases) for your platform, or build from source:

```bash
# Pre-built binary (macOS)
# Download from Releases page, extract, and run
./memfuse-server

# Or run from this source checkout
./run-server.sh
```

This source checkout uses port **18720** for repo-local development. Verify:

```bash
curl http://127.0.0.1:18720/health
```

Without API keys, MemFuse runs in **deterministic mode** (regex facts, keyword search, Jaccard matching) and never blocks. Set `MEMFUSE_OPENAI_API_KEY` or `MEMFUSE_JINA_API_KEY` for richer LLM-assisted extraction.

### 2. Install the SDK

```bash
npm install @percena/memfuse
```

### 3. Set Up for Your Agent Platform

```bash
# Claude Code
npx memfuse-setup install --platform=claude-code --server-url=http://127.0.0.1:18720

# Codex
npx memfuse-setup install --platform=codex --server-url=http://127.0.0.1:18720
```

That's it. MemFuse hooks into the agent lifecycle transparently — no explicit "use memfuse" commands needed.

### 4. Optional: Install as a System Service

```bash
npx memfuse service install
npx memfuse service start
npx memfuse service doctor
```

**Core configuration:**

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMFUSE_WORKSPACE_ROOT` | `~/.memfuse/data` | Data root (SQLite, sessions, projected resources) |
| `MEMFUSE_OPENAI_API_KEY` | — | Enables LLM-assisted extraction (OpenAI-compatible) |
| `MEMFUSE_JINA_API_KEY` | — | Enables semantic search (Jina embeddings) |

---

## How It Works

### The Memory Pipeline

```
Session starts
  → SessionStart hook injects relevant facts, episodes, and signals
  → Agent works normally (reads files, runs tools, writes code)
  → PostToolUse hook captures each tool execution as an observation
  → UserPromptSubmit hook injects per-prompt context (Claude Code)
  → PreToolUse[Read] hook shows memory hints before file reads
  → Session ends
  → Background pipeline: chunk episodes → extract facts → update heuristics → consolidate
  → Next session starts with richer context
```

### Signal-Beacon Philosophy

MemFuse doesn't try to be an encyclopedia. For code repositories, it generates:

- **L0 Abstracts** (~50 tokens) — *"This file handles OAuth token refresh with retry logic"*
- **L1 Overviews** (~200 tokens) — Key exports, dependencies, design decisions
- **L2 Full Content** — Available on demand, not injected by default

High-level memory (L0/L1) updates rarely and travels light. Detail memory (L2) stays local and updates with the code. They're stored in separate tables, ready for future physical split (cloud vs local).

### Memory Types

| Type | What | Lifecycle |
|------|------|-----------|
| **Facts** | Structured subject-predicate-value triples with confidence | Active → superseded → retracted |
| **Episodes** | Chunked, summarized interaction segments | Decay by forgetting curve, reinforced on recall |
| **Heuristics** | Learned behavioral rules from user patterns | Draft → candidate → confirmed |
| **Observations** | Raw tool execution captures | Consolidated into episodes |
| **Briefs** | Cross-session summaries per resource/user | Rebuilt on consolidation |

---

## Agent Integration

### Claude Code (8 hooks + 43 MCP tools + Skill)

| Layer | What It Does |
|-------|-------------|
| **SessionStart** | Inject facts, episodes, signals from prior sessions |
| **UserPromptSubmit** | Lightweight per-prompt context injection |
| **PreToolUse[Read]** | Memory hints before file reads |
| **PostToolUse** | Capture tool execution observations |
| **Stop** | Generate session turn summary |
| **PreCompact** | Snapshot context before host compaction |
| **SessionEnd** | Trigger background consolidation pipeline |
| **Setup** | Health check at startup |

### Codex (3 hooks + 43 MCP tools + Skill)

| Layer | What It Does |
|-------|-------------|
| **SessionStart** | Context injection |
| **PostToolUse** | Observation capture |
| **Stop** | Summary + commit |

### Any Application (REST API)

Any app can integrate via the HTTP API:

```bash
# Store a memory
curl -X POST http://localhost:18720/sessions/{id}/observations \
  -H "Content-Type: application/json" \
  -d '{"tool_name":"user_note","content":"Prefer dark mode in all UIs"}'

# Recall relevant context
curl -X POST http://localhost:18720/context/resolve \
  -H "Content-Type: application/json" \
  -d '{"query":"UI preferences","user_id":"default"}'

# Search memories
curl -X POST http://localhost:18720/v1/memory:search \
  -H "Content-Type: application/json" \
  -d '{"query":"authentication decisions","limit":5}'
```

### Any MCP-Compatible Agent

```json
{
  "mcpServers": {
    "memfuse": {
      "command": "npx",
      "args": ["memfuse-mcp"],
      "env": {
        "MEMFUSE_SERVER_URL": "http://localhost:18720",
        "MEMFUSE_USER_ID": "your-user-id"
      }
    }
  }
}
```

---

## Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                      Any Agent / Application                     │
├──────────────────────┬──────────────────────┬───────────────────┤
│   Claude Code        │   Codex / Cursor     │   REST API        │
│   (8 Hooks + MCP)    │   (3 Hooks + MCP)    │   (HTTP Client)   │
└──────────┬───────────┴──────────┬───────────┴────────┬──────────┘
           │                      │                    │
           ▼                      ▼                    ▼
┌─────────────────────────────────────────────────────────────────┐
│                @percena/memfuse (TypeScript)                      │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │  MCP Server  │  │  Lifecycle   │  │  LOOK→DIG→SAVE       │  │
│  │  (43 tools)  │  │  Hooks       │  │  Skill               │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
└────────────────────────────┬────────────────────────────────────┘
                             │ HTTP
                             ▼
┌─────────────────────────────────────────────────────────────────┐
│              MemFuse Server (Rust — single binary)              │
│                    Port 18720 · 110+ API endpoints              │
│                                                                 │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────────────┐  │
│  │  Memory      │  │  Retrieval   │  │  Consolidation       │  │
│  │  Pipeline    │  │  Engine      │  │  ("Dream" loop)      │  │
│  └──────────────┘  └──────────────┘  └──────────────────────┘  │
│                                                                 │
│  SQLite: metadata (facts, episodes, sessions)                   │
│        + tiered semantic index (L0/L1 high · L2+ detail)        │
└─────────────────────────────────────────────────────────────────┘
```

### Crate Structure (17 Rust crates)

| Crate | Responsibility |
|-------|---------------|
| `mfs-server` | Axum HTTP server, 110+ REST API endpoints |
| `mfs-memory` | Core business logic: extraction, episodes, facts, intent, render |
| `mfs-planning` | Active Overlay state machine, conflict detection |
| `mfs-session` | Session lifecycle, background memory pipeline |
| `mfs-ops` | Resource operations: ingest, refresh, rebuild, export/import |
| `mfs-retrieval` | Multi-strategy retrieval engine (vector + rerank + MMR) |
| `mfs-index` | Tiered semantic search index (FTS5 + embeddings, high-level/detail split) |
| `mfs-semantic` | LLM providers (OpenAI-compatible), embedding, circuit breaker |
| `mfs-metadata` | SQLite persistence layer |
| `mfs-workspace` | Workspace filesystem operations |
| `mfs-connectors` | External connectors (localfs, git, url) |
| `mfs-ast` | AST skeleton extraction (Rust, Python, TS, JS, Go, Java, C/C++) |
| `mfs-cli` | CLI binary (~50 commands) |
| `mfs-mcp` | Internal MCP adapter (33 ops tools) |
| `mfs-types` | Shared error and domain types |
| `mfs-uri` | `mfs://` URI parsing and resolution |
| `mfs-test-util` | Test environment helpers |

---

## Resource Import

```bash
# Import a local directory
curl -X POST -H "Content-Type: application/json" \
  -d '{"source_kind":"localfs","source_path":"/path/to/docs","logical_name":"my-docs"}' \
  http://127.0.0.1:18720/resources

# Import a Git repo
curl -X POST -H "Content-Type: application/json" \
  -d '{"source_kind":"git","source_path":"/path/to/repo","logical_name":"my-repo"}' \
  http://127.0.0.1:18720/resources

# Import inline content
curl -X POST -H "Content-Type: application/json" \
  -d '{"file_name":"notes.md","content":"Meeting notes...","logical_name":"notes"}' \
  http://127.0.0.1:18720/resources
```

Git resources auto-discover host, namespace, repo, and ref from the remote origin.

---

## Configuration

### Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `MEMFUSE_WORKSPACE_ROOT` | `~/.memfuse/data` | Data root (SQLite, sessions, projected resources) |
| `MEMFUSE_BIND_ADDR` | `127.0.0.1:18720` | Repo-local server bind address |
| `MEMFUSE_SERVER_URL` | `http://127.0.0.1:18720` | SDK: repo-local server URL |
| `MEMFUSE_USER_ID` | `default` | User identifier |
| `MEMFUSE_OPENAI_API_KEY` | — | OpenAI API Key (enables LLM extraction; `OPENAI_API_KEY` also works as fallback) |
| `MEMFUSE_OPENAI_API_BASE` | `https://api.openai.com/v1` | Compatible LLM endpoint (`OPENAI_BASE_URL` also works as fallback) |
| `MEMFUSE_JINA_API_KEY` | — | Jina Embedding Key (enables semantic search) |
| `MEMFUSE_TLS_INSECURE` | — | Set to any value to skip HTTPS certificate verification |
| `MEMFUSE_DREAM_MIN_HOURS` | `24` | Auto-consolidation: minimum hours between runs |
| `MEMFUSE_DREAM_MIN_SESSIONS` | `5` | Auto-consolidation: minimum committed sessions |
| `MEMFUSE_DREAM_POLL_SECS` | `300` | Auto-consolidation: polling interval in seconds |

See `.env.example` for the full list.

---

## Data Storage

All data lives in `{MEMFUSE_WORKSPACE_ROOT}/` using embedded SQLite — no external database.

```
{workspace_root}/
├── _system/
│   ├── metadata.sqlite     ← Facts, episodes, sessions, cursors, audit
│   └── semantic.sqlite     ← Tiered index: high-level (L0/L1) + detail (L2+)
└── tenants/{account}/{user}/
    ├── resources/           ← Projected resource files with L0/L1/L2 layers
    ├── user/memories/       ← Profile, preferences, entities, events
    ├── agent/{id}/memories/ ← Patterns, skills, cases, tools
    └── session/             ← Session archives + messages.jsonl
```

### Viewing Data

```bash
# Via API
curl http://127.0.0.1:18720/facts?user_id=default
curl http://127.0.0.1:18720/system/status

# Via SQLite directly
sqlite3 {workspace_root}/_system/metadata.sqlite \
  "SELECT predicate, display_value, confidence FROM facts WHERE status='active'"
```

---

## API Reference

### Core Memory

| Method | Path | Description |
|--------|------|-------------|
| POST | `/context/resolve` | Resolve and inject memory context |
| POST | `/v1/memory:search` | Search memories (precision / diverse / recent / comprehensive) |
| GET | `/facts` | List active facts |
| POST | `/memories/cite` | Reinforce useful memories (increment recall_count) |
| GET | `/memories/export` | Export as editable Markdown |
| POST | `/memories/import` | Import from Markdown |

### Sessions

| Method | Path | Description |
|--------|------|-------------|
| POST | `/sessions` | Create session |
| POST | `/sessions/{id}/observations` | Store observation |
| POST | `/sessions/{id}/commit` | Commit session (trigger consolidation) |

### Resources

| Method | Path | Description |
|--------|------|-------------|
| POST | `/resources` | Register resource (localfs / git / inline) |
| POST | `/resources/{id}/refresh` | Re-ingest after source changes |
| POST | `/resources/{id}/rebuild` | Rebuild semantic index |

### Navigation

| Method | Path | Description |
|--------|------|-------------|
| GET | `/ls?uri=` | List directory |
| GET | `/read?uri=` | Full content (L2) |
| GET | `/abstract?uri=` | Summary (L0) |
| GET | `/overview?uri=` | Overview (L1) |
| GET | `/search?query=` | Semantic search |
| GET | `/grep?query=` | Keyword grep |

110+ total endpoints — see [docs/architecture.md](docs/architecture.md) for the complete list.

---

## CLI

The Node.js CLI (`memfuse`) provides 110 commands covering all API operations. The Skill uses it under the hood.

```bash
# Search memories
npx memfuse search --query "auth decisions" --strategy diverse

# Inspect a resource
npx memfuse abstract --uri mfs://resources/git/github.com/org/repo

# List facts
npx memfuse list-facts

# Store an observation
npx memfuse store-observation --tool-name "discovery" --content "Found rate limiter config in gateway/"

# Check system health
npx memfuse health
```

Or install globally for shorter commands:

```bash
npm install -g @percena/memfuse
memfuse search --query "auth decisions"
```

The Rust CLI (`mfs-cli`) provides ~50 offline/diagnostic commands for direct workspace access when the server is unavailable.

---

## Development

### Building

```bash
# Rust server
cargo build --release -p mfs-server

# SDK
cd sdk && npm install && npm run build
```

### Testing

```bash
# All Rust tests (~750 tests)
cargo test

# SDK tests
cd sdk && node --test tests/sdk.test.mjs
```

---

## Troubleshooting

| Problem | Solution |
|---------|----------|
| Server won't start | Check `MEMFUSE_WORKSPACE_ROOT` is set and directory exists |
| Hooks not triggering | Claude Code: check `.claude/settings.local.json`; Codex: ensure hooks are trusted |
| No semantic search | Set `MEMFUSE_JINA_API_KEY` or `OPENAI_API_KEY` for embedding generation |
| Self-signed cert errors | Set `MEMFUSE_TLS_INSECURE=1` to skip certificate verification |
| Deterministic mode | Without LLM keys: 45 regex rules, keyword search, Jaccard matching — functional but less rich |

---

## Roadmap

### Planned

- **Vector search acceleration** — ANN index (HNSW / IVF) for sub-linear similarity search
- **Web UI / Dashboard** — Browse facts, episodes, heuristics; manage memory lifecycle
- **More agent platforms** — Cursor, Windsurf, and other coding agents
- **Local embedding inference** — ONNX Runtime for on-device embedding, eliminating API dependency

### Future

- **Cloud sync** — L0/L1 high-level memory synced to cloud for cross-device continuity
- **Multi-device federation** — Merge and reconcile memories from multiple machines
- **PostgreSQL backend** — Optional PgSQL for team/enterprise deployments
- **Personalized mental model** — Adaptive memory that converges to your thinking patterns over time

---

## Acknowledgments

MemFuse is inspired by and draws design ideas from the following projects:

- [OpenViking](https://github.com/volcengine/OpenViking) by Volcengine — inspired the signal-beacon memory architecture and tiered information retrieval approach.

## License

MIT License © 2026 Percena
