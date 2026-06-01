# MemFuse Architecture Document

> Status: Active
> Last updated: 2026-06-01
> This document describes the current Rust + SDK architecture.

## 1. System Overview

```
┌─── Application Layer ───────────────────────────────────────────────┐
│                                                                       │
│  Claude Code ──┐                                                     │
│  Codex ────────┤   @percena/memfuse (TypeScript npm package)       │
│  Clipper ──────┤   MCP Server + Hooks + Skills + Client + Adapters  │
│  Other ────────┘                                                     │
│                                                                       │
└──────────────────────────┬──────────────────────────────────────────┘
                           │ HTTP / stdio MCP
                           ▼
┌─── MemFuse Server (Rust, single binary) ────────────────────────────┐
│                                                                       │
│  ┌─ mfs-server (HTTP API) ────────────────────────────────────┐    │
│  │  Axum routes (see §3.2 for full list)                      │    │
│  │  MCP endpoint (optional sidecar for stdio)                 │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌─ mfs-memory (Memory Business Logic) ──────────────────────┐    │
│  │  overlay     │ budget      │ intent      │ facts           │    │
│  │  episodes    │ consolidation │ briefs   │ render          │    │
│  │  candidates  │ writeback   │ commit service │ maintenance │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌─ mfs-planning (Plan / Overlay Business Logic) ─────────────┐    │
│  │  Active Overlay state machine │ conflict detection         │    │
│  │  canonical Canvas refs        │ idempotency                │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌─ Storage Engine (10 crates) ──────────────────────────┐    │
│  │  mfs-session  │ mfs-workspace │ mfs-metadata             │    │
│  │  mfs-connectors│ mfs-semantic │ mfs-retrieval             │    │
│  │  mfs-index    │ mfs-ast      │ mfs-uri  │ mfs-types       │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  ┌─ mfs-mcp (Internal MCP stdio) ─────────────────────────────┐    │
│  │  33 management/ops tools (session, resource, watch,       │    │
│  │  skill, relation, system, task)                            │    │
│  │  (Developer/ops tool, NOT exposed to Agent users)          │    │
│  └────────────────────────────────────────────────────────────┘    │
│                                                                       │
└──────────────────────────────────────────────────────────────────────┘

Data on disk:
  {workspace_root}/                        ← MEMFUSE_WORKSPACE_ROOT
    ├── _system/
    │   ├── metadata.sqlite                ← Resource registry, path entries, audit, snapshots, facts, tasks
    │   └── semantic.sqlite                ← Tiered FTS5 + vec0 vector index (high-level / detail split)
    └── tenants/{account}/{user}/          ← mfs:// URI physical projection
        ├── resources/                     ← Connector-backed resources (localfs, git)
        ├── user/memories/                 ← User-owned memories (8 categories)
        │   ├── profile/ preferences/ entities/ events/
        ├── agent/{id}/memories/           ← Agent-owned memories
        │   ├── patterns/ skills/ cases/ tools/
        └── session/                       ← Session archives + messages.jsonl
```

## 2. Crate Dependency Graph

```
Binary crates (top-level, depend on many library crates):

mfs-server   ← mfs-ast, mfs-connectors, mfs-index, mfs-metadata, mfs-memory,
                mfs-ops, mfs-planning, mfs-retrieval, mfs-semantic, mfs-session,
                mfs-types, mfs-uri, mfs-workspace
mfs-cli      ← mfs-metadata, mfs-memory, mfs-ops, mfs-retrieval, mfs-session,
                mfs-types, mfs-workspace
mfs-mcp      ← mfs-metadata, mfs-memory, mfs-ops, mfs-retrieval, mfs-semantic,
                mfs-session, mfs-types, mfs-uri, mfs-workspace

Library crates (internal dependencies):

mfs-ops         ← mfs-ast, mfs-connectors, mfs-index, mfs-metadata, mfs-retrieval,
                   mfs-semantic, mfs-session, mfs-types, mfs-uri, mfs-workspace
mfs-session     ← mfs-index, mfs-memory, mfs-metadata, mfs-semantic, mfs-types,
                   mfs-uri, mfs-workspace
mfs-memory      ← mfs-metadata, mfs-semantic, mfs-types, mfs-uri
mfs-retrieval   ← mfs-index, mfs-metadata, mfs-semantic, mfs-types, mfs-workspace
mfs-workspace   ← mfs-ast, mfs-connectors, mfs-metadata, mfs-types, mfs-uri
mfs-semantic    ← mfs-ast, mfs-index, mfs-types
mfs-planning    ← mfs-metadata, mfs-types
mfs-index       ← mfs-types
mfs-uri         ← mfs-types

Leaf crates (no internal deps):

mfs-types       ← thiserror, regex (MfsError + IdentityContext)
mfs-metadata    ← rusqlite (SQLite persistence)
mfs-connectors  ← git2, reqwest (localfs, git, url sources)
mfs-ast         ← standalone (language detection + regex AST extraction)
```

## 3. API Design

### 3.1 Naming Conventions

| Convention | Rule | Industry Reference |
|-----------|------|--------------------|
| HTTP paths | `/v1/{resource}`, `/v1/{resource}/{id}:{action}` | Google API Design Guide, Stripe |
| JSON fields | `snake_case` | Python/REST convention, Anthropic API, OpenAI API |
| MCP tool names | `snake_case` (preserved from current) | MCP spec community pattern |
| Env vars | `MEMFUSE_` prefix, `UPPER_SNAKE_CASE` | 12-factor app, PostgreSQL `PG_` pattern |
| URI scheme | `mfs://` | RFC 3986 custom scheme convention |
| Rust crates | `mfs-{module}` (kebab-case) | Rust API Guidelines (C-CRATE-NAME) |
| Rust types | `PascalCase` structs, `snake_case` functions | Rust API Guidelines (C-NAMING) |
| Rust modules | `snake_case` | Rust API Guidelines (C-MOD-NAMING) |
| Prometheus metrics | `memfuse_{unit}_{suffix}` | Prometheus naming conventions |
| npm package | `@percena/memfuse` | npm scoped package convention |

### 3.2 HTTP API Paths

All paths are registered in `crates/mfs-server/src/http.rs` via Axum. The server uses flat, unprefixed paths for most routes — no `/mcp/v1/`, `/v1/`, or `/v2/` prefix for the core memory pipeline.

**Memory pipeline routes** (used by SDK hooks and MCP server):

| Method | Path | Purpose | SDK Mapping |
|--------|------|---------|-------------|
| POST | `/sessions/{id}/observations` | Add observation (post-tool-use / stop / pre-compact hooks) | `store_observation` |
| POST | `/sessions/{id}/commit` | Commit session (session-end hook) | SessionEnd hook |
| GET | `/episodes/{episode_id}` | Episode detail payload | `get_observations` |
| GET | `/episodes/{episode_id}/timeline` | Episodic neighborhood around an anchor episode | `timeline` |
| POST | `/context/resolve` | Resolve memory context (session-start / pre-compact) | `resolve_context` |
| POST | `/v1/memory:search` | Search memories (MCP search tool); supports `strategy` param: precision (default, semantic+rerank), diverse (semantic+rerank+MMR), recent (time-weighted), comprehensive (full LLM classification+broad retrieval) | `search_memories` |
| GET | `/facts` | List facts (MCP list_facts tool) | `list_facts` |
| GET | `/facts?at_time=...` | List facts effective at a point in time (MCP facts_at_time tool) | `facts_at_time` |
| GET | `/facts/{fact_id}/trace` | Fact provenance trace (assertion → extraction → source turns) | `trace_fact` |
| GET | `/health` | Health check | SDK `checkHealth` |
| POST | `/memories/cite` | Citation feedback (increment recall_count) | `cite_memories` |
| GET | `/memories/export` | Export memories as editable Markdown | `export_memories` |
| POST | `/memories/import` | Import memories from Markdown | `import_memories` |
| POST | `/v1/eval/recall` | Evaluate recall accuracy (benchmark tool) | `eval_recall` |
| POST | `/v1/memory:archive` | Archive (soft-delete) memories by scope or age | `archive_memories` |

Commit semantics:

- `POST /sessions/{id}/commit` archives the active session through `mfs-session`
- the background task preserves existing workspace memory outputs
- the same task also persists canonical `mfs-memory` entities into `metadata.sqlite`

**Session management routes:**

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/sessions` | Create session |
| GET | `/sessions` | List sessions |
| GET | `/sessions/{id}` | Get session |
| DELETE | `/sessions/{id}` | Delete session |
| POST | `/sessions/{id}/messages` | Add message (user/assistant turn) |
| GET | `/sessions/{id}/context` | Get session context |
| GET | `/sessions/{id}/timeline` | Archive/session timeline |
| POST | `/sessions/{id}/commit` | Commit session (trigger consolidation) |
| GET | `/sessions/{id}/archives/{archive_id}` | Get session archive |

**Resource management routes:**

| Method | Path | Purpose |
|--------|------|---------|
| POST | `/resources` | Create resource |
| GET | `/resources` | List resources |
| POST | `/resources/{id}/export` | Export resource pack |
| POST | `/resources/{id}/watch` | Register resource watch |
| POST | `/resources/{id}/refresh` | Refresh resource projection |
| POST | `/resources/{id}/rebuild` | Rebuild resource |

**Storage engine routes** (direct filesystem/memory operations):

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/read?uri=` | Read file content (L2) |
| GET | `/abstract?uri=` | Read short summary (L0) |
| GET | `/overview?uri=` | Read overview (L1) |
| GET | `/ls?uri=` | List directory |
| GET | `/tree?uri=` | Directory tree |
| GET | `/search?query=` | Direct search (bypass memory logic) |
| GET | `/find?uri=` | Find files |
| GET | `/grep?uri=` | Grep search |
| GET | `/glob?uri=&pattern=` | Glob pattern search |
| GET | `/stat?uri=` | File/directory stat |
| POST | `/write` | Write file to workspace |
| POST | `/mkdir` | Create directory |
| POST | `/mv` | Move file |
| DELETE | `/rm?uri=` | Remove file |

**Internal/management routes:**

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/health` | Health check |
| GET | `/ready` | Readiness check |
| GET | `/metrics` | Prometheus metrics |
| GET | `/system/status` | System status |
| GET | `/system/observer` | Observer status |
| GET | `/tasks` | List tasks |
| GET | `/tasks/{id}` | Get task status |
| GET | `/tasks/{id}/wait` | Wait for task completion |

> 完整路由列表见 README.md API Reference 章节及 [user-guide.md](user-guide.md)

### 3.3 MCP Tool Schema

The MCP server in the SDK calls the MemFuse Server HTTP API. All paths map directly to Rust Server Axum routes — no `/mcp/v1/` prefix.

| Tool | Method | Server Path | Input |
|------|--------|------------|-------|
| `memfuse_guide` | (local) | None | `{ topic?: string }` |
| `search_memories` | POST | `/v1/memory:search` | `{ query: string, limit?: number }` |
| `timeline` | GET | `/episodes/{episode_id}/timeline` | `{ episode_id: string, direction?: "before"|"after"|"both", radius?: number }` |
| `get_observations` | GET | `/episodes/{episode_id}` | `{ episode_ids: string[] }` |
| `resolve_context` | POST | `/context/resolve` | `{ session_id: string, query: string, token_budget?: number }` |
| `store_observation` | POST | `/sessions/{sessionId}/observations` | `{ tool_name: string, tool_input?: string, tool_output?: string, content?: string }` |
| `list_facts` | GET | `/facts?user_id=...` | `{ }` |

> 完整工具列表见 [user-guide.md §5](user-guide.md)（共 43 个工具）

### 3.4 JSON Response Format Convention

Most current API responses are direct JSON payloads rather than a global envelope. Error responses follow:

```json
{
  "error": {
    "category": "NotFound",
    "message": "Episode not found",
    "retryable": false
  }
}
```

MCP responses remain in MCP SDK format: `{ content: [{ type: 'text', text: <markdown> }] }`.

## 4. Data Model

### 4.1 Core Entities

```
User
  ├── Session (1:N)
  │    ├── Turn (1:N) ← role: user/assistant/observation/system
  │    └── Archive (1:N) ← committed snapshot of messages
  │
  ├── Fact (1:N) ← projected from assertions
  │    ├── predicate: string (e.g. "location.current_city")
  │    ├── value_type: scalar | set | temporal
  │    ├── status: active | superseded | retracted | expired
  │    ├── confidence: float (0.0-1.0)
  │    ├── recall_count: int
  │    ├── last_recalled_at: timestamp
  │    └── valid_from / valid_to (temporal validity: active on creation; valid_to populated on supersede/retract)
  │
  ├── Episode (1:N) ← chunked from turns
  │    ├── summary: string
  │    ├── salience_score: float
  │    ├── strength_score: float
  │    ├── recall_count: int
  │    ├── last_recalled_at: timestamp
  │    └── source_turn_ids: string[]
  │
  ├── MemoryBrief (1:N) ← cross-thread summary
  │    ├── scope: resource | user
  │    ├── summary_text: string
  │    ├── source_threads: string[]
  │
  ├── ConsolidationCursor (1:1) ← per user, tracks last processed turn
  │
  └── ResourceSource (1:N) ← registered resource (table: resource_sources)
       ├── source_kind: localfs | git | inline
       ├── source_identifier: string
       ├── canonical_root_uri: string
       ├── last_snapshot_id: string
```

### 4.2 Fact Predicate Taxonomy

| Predicate | Value Type | Category | Examples |
|-----------|-----------|----------|---------|
| `identity.name` | scalar | Profile | "User's name is Alice" |
| `identity.pronouns` | scalar | Profile | "User uses she/her pronouns" |
| `location.current_city` | scalar | Profile | "User currently lives in Tokyo" |
| `location.current_country` | scalar | Profile | "User is based in Japan" |
| `language.spoken` | set | Profile | "User speaks English, Chinese" |
| `work.current_role` | scalar | Profile | "User is a software engineer" |
| `work.current_company` | scalar | Profile | "User works at Acme Corp" |
| `project.active` | set | Entities | "User is working on Project Alpha" |
| `entities.architecture_decision` | scalar | Entities | "Switched from REST to GraphQL for API layer" |
| `health.allergy` | set | Preferences | "User is allergic to peanuts" |
| `health.constraint` | set | Preferences | "User must avoid gluten" |
| `diet.spicy_preference` | scalar | Preferences | "User prefers medium spicy food" |
| `preference.food` | set | Preferences | "User likes sushi, ramen" |
| `preference.communication_style` | scalar | Preferences | "User prefers concise explanations" |
| `procedure.build_command` | scalar | Procedural | "Project builds with `cargo build`" |
| `procedure.test_command` | scalar | Procedural | "Tests run via `cargo test`" |
| `procedure.deploy_step` | scalar | Procedural | "Deploy with `docker compose up`" |
| `convention.tool` | scalar | Procedural | "Project uses ESLint for linting" |
| `convention.naming` | scalar | Procedural | "Files follow snake_case naming" |
| `environment.ci` | scalar | Procedural | "CI pipeline runs on GitHub Actions" |
| `environment.runtime` | scalar | Procedural | "Runtime target is Node 20 LTS" |

> 上表列出部分声明性谓词及程序性谓词示例。当前实际共21个谓词（含 procedure.build_command, procedure.test_command, procedure.deploy_step, convention.tool, convention.naming, environment.ci, environment.runtime 等7个程序性谓词），45条提取规则。完整规则列表见 `mfs-memory` crate `facts` 模块。

### 4.3 Memory Categories (from storage engine)

| Category | Ownership | Mergeable | Path Pattern |
|----------|-----------|-----------|-------------|
| profile | user | yes | `mfs://user/memories/profile.md` |
| preferences | user | yes | `mfs://user/memories/preferences/general.md` |
| entities | user | yes | `mfs://user/memories/entities/{title}.md` |
| events | user | no (append) | `mfs://user/memories/events/{title}-{content}.md` |
| cases | agent | no (append) | `mfs://agent/{id}/memories/cases/{title}-{content}.md` |
| patterns | agent | yes | `mfs://agent/{id}/memories/patterns/{title}.md` |
| tools | agent | yes | `mfs://agent/{id}/memories/tools/{title}.md` |
| skills | agent | yes | `mfs://agent/{id}/memories/skills/{title}.md` |

### 4.4 Session Archive Structure

```
mfs://session/{session_id}/
  ├── messages.jsonl              ← Full conversation log
  ├── history/
  │    └── archive_001/
  │        ├── messages.jsonl     ← Archived messages
  │        ├── .abstract.md       ← L0 summary
  │        ├── .overview.md       ← L1 overview
  │        └── .done              ← Completion marker
  ├── recent-turns.json           ← Last N turns (overlay source)
  ├── continuity.json             ← Session state for context injection
  └── session.json                ← Session metadata
```

## 5. SDK Architecture

### 5.1 Package Structure

```
@percena/memfuse/
  ├── src/
  │    ├── index.ts                ← Public API exports
  │    ├── mcp/
  │    │    └── server.ts          ← MCP stdio server (43 tools, inline definitions)
  │    ├── hooks/
  │    │    ├── index.ts           ← Hook exports
  │    │    ├── session-start.ts
  │    │    ├── pre-tool-use.ts
  │    │    ├── post-tool-use.ts
  │    │    ├── user-prompt-submit.ts
  │    │    ├── stop.ts
  │    │    ├── pre-compact.ts
  │    │    ├── session-end.ts
  │    │    ├── setup.ts
  │    │    └── platform-utils.ts  ← Platform detection helpers
  │    ├── cli/
  │    │    ├── index.ts           ← Command router + dispatch
  │    │    ├── output.ts          ← 3-mode formatter (default/json/verbose)
  │    │    ├── types.ts           ← CliArgs, helpers
  │    │    └── commands/          ← 16 command modules (core, dig, facts, session, ...)
  │    ├── client/
  │    │    ├── http.ts            ← HTTP transport (fetch + retry)
  │    │    ├── runtime.ts         ← Runtime client types
  │    │    ├── runtime-client.ts  ← Runtime client implementation
  │    │    ├── ops-client.ts      ← Ops client (administrative)
  │    │    ├── adapter.ts         ← Client adapter interface
  │    │    ├── render.ts          ← Context/episode rendering
  │    │    ├── types.ts           ← TypeScript type definitions
  │    │    ├── index.ts           ← Client exports
  │    │    └── adapters/
  │    │         ├── claude-code.ts
  │    │         ├── codex.ts
  │    │         └── generic.ts
  │    ├── shared/
  │    │    ├── config.ts          ← Shared configuration
  │    │    ├── http.ts            ← Shared HTTP utilities
  │    │    ├── router.ts          ← Backend routing
  │    │    ├── utils.ts           ← General utilities
  │    │    └── privacy.ts         ← Privacy/sanitization
  │    ├── service/
  │    │    └── manager.ts         ← Service lifecycle management
  │    ├── skills/
  │    │    ├── index.ts
  │    │    ├── loader.ts          ← Skill markdown loader
  │    │    └── memfuse/
  │    │         ├── SKILL.md
  │    │         └── references/commands.md
  │    └── setup/
  │         ├── install.ts         ← Platform setup (hooks, MCP, skills)
  │         └── uninstall.ts       ← Cleanup command
  │
  ├── bin/
  │    ├── memfuse.cjs             ← CLI entry point
  │    ├── memfuse-mcp.cjs         ← MCP stdio entry point
  │    └── memfuse-setup.cjs       ← Setup entry point
  │
  ├── package.json
  └── tsconfig.json
```

### 5.2 Hook Routing

All hooks use **single-backend routing** — direct HTTP calls to the Rust Server with no dual-backend fallback.

```
callServer(method, path, body)
  → MEMFUSE_SERVER_URL + path  (single server, single path set)
```

All paths are the canonical MemFuse API paths (Section 3.2), matching the Rust Server Axum routes directly.

### 5.3 MCP Server Entry Point

The MCP server runs as a stdio process, launched by the Agent framework:

```bash
node @percena/memfuse/bin/memfuse-mcp.cjs
```

Environment variables consumed:
- `MEMFUSE_SERVER_URL` (required)
- `MEMFUSE_USER_ID` (optional, default: `USER` env var or `default`)
- `MEMFUSE_THREAD_ID` (optional)

### 5.4 Hook Entry Points

Each hook is a standalone script that reads stdin JSON, processes it, and writes stdout:

```bash
memfuse session-start    # via SDK CLI bin entry
memfuse post-tool-use    # (hooks are invoked by Agent platform, not directly by users)
...
```

All hooks share a common `platform.ts` module for detection and adaptation, and a `routing.ts` module for Server HTTP calls.

## 6. Server Architecture

### 6.1 Request Processing Pipeline

```
HTTP Request → Axum Router
  → Handler (extract params, validate)
  → mfs-memory service (business logic)
    → overlay.filter()
    → budget.allocate()
    → intent.route()
    → facts.extract() / facts.project()
    → episodes.chunk() / episodes.search()
    → briefs.build()
    → render.inject()
  → mfs-session service (data operations)
    → session.add_message()
    → memory.extract() / memory.merge()
    → retrieval.search()
    → metadata.query()
  → Response serialization
```

### 6.2 Consolidation Pipeline (async, triggered by commit)

```
Session commit → spawn background task
  → cursor.resolve_window()      ← Find unprocessed turns
  → chunking.split()             ← Split into episodes
  → episodes.build()             ← Summarize + embed
  → facts.extract()              ← Regex rule matching
  → facts.project()              ← Conflict resolution → write to DB
  → briefs.refresh()             ← Update cross-thread summaries
  → cursor.advance()             ← Mark turns as processed
```

### 6.3 Data Flow: resolve_context (the primary Agent read path)

```
resolve_context(query, budget)
  1. build_overlay_entries(session_id)
     → fetch turns after cursor, filter by role + confirmation phrases, cap 6 entries / 350 tokens
  2. load_and_transform_facts(user_id)
     → load facts where valid_to IS NULL (current active facts only), FTS5 lexical boost
  3. load_and_transform_episodes(user_id)
     → load recent episodes for context
  4. classify_intent(query)
     → LLM-assisted intent classification (Comprehensive strategy) or keyword fallback
  5. plan_section_budgets(overlay_tokens, remaining)
     → overlay takes actual cost, remaining split between facts and episodes
  6. filter_facts / route_facts_for_intent
     → if intent matched: route facts by predicate prefix; otherwise: filter by confidence
     → cap_facts_by_budget
  7. embed_query + rerank_episodes_with_query
     → cosine similarity scoring → rerank top candidates
     → if strategy="diverse": apply MMR diversity post-processing
     → tie-break: last_recalled_at (reinforce) > strength > created_at
  8. retrieve_heuristics
     → load confirmed/draft heuristic rules matching query context
  9. render_memory_injection(overlay, facts, episodes, heuristics)
     → [Current Facts] / [Recent Updates] / [Relevant History] / [Behavioral Heuristics]
 10. writeback_recall_and_access_log
     → increment recall_count / last_recalled_at on recalled items; append access log
```

## 7. Separation of Concerns

### 7.1 mfs-memory vs storage crates boundary

**mfs-memory** owns: What to remember, how to remember it, and how to present it.
**Storage crates** (mfs-session, mfs-metadata, mfs-retrieval, etc.) own: Where to store it, how to index it, and how to retrieve raw data.

| Capability | Owner | Reason |
|-----------|-------|--------|
| Overlay filtering logic | mfs-memory | Business rule: which unconsolidated turns to show |
| Budget allocation | mfs-memory | Business rule: how much context to inject |
| Fact predicate taxonomy | mfs-memory | Business rule: which facts matter and their lifecycle |
| Episode salience scoring | mfs-memory | Business rule: how to rank episodic relevance |
| Consolidation cursor tracking | mfs-memory | Business rule: incremental processing boundary |
| Compaction state machine | mfs-memory | Business rule: when to compress session history |
| Memory injection rendering | mfs-memory | Business rule: how to format context for Agent |
| Session archive writing | mfs-session | Storage: serialize messages to disk |
| Memory file writing (8-cat) | mfs-session | Storage: write .md files to workspace |
| Vector embedding generation | mfs-semantic | Storage: generate and store embeddings |
| Hierarchical retrieval | mfs-retrieval | Storage: L0→L1→L2 search infrastructure |
| Resource connector materialization | mfs-connectors | Storage: fetch external content to local disk |
| SQLite metadata CRUD | mfs-metadata | Storage: persistent data management |
| URI path mapping | mfs-uri | Storage: mfs:// → filesystem path resolution |

### 7.2 mfs-mcp vs SDK MCP boundary

| MCP Server | Scope | Audience | Transport |
|-----------|-------|----------|-----------|
| SDK MCP (in @percena/memfuse) | 43 Agent-facing tools | Agent users | stdio (per-session) |
| mfs-mcp (in crate) | 33 management/ops tools (session, resource, watch, skill, relation, system, task) | Developers/ops | stdio (optional) |

The SDK MCP server calls MemFuse Server HTTP API. The mfs-mcp crate calls storage engine directly (no HTTP). These two MCP surfaces serve different audiences and do not overlap in tool names or functionality.

## 8. Deployment

### 8.1 Build and Run

```bash
# Build server
cargo build --release -p mfs-server

# Start server (development)
./run-server.sh

# Start server (standalone binary)
./target/release/mfs-server --bind-addr 127.0.0.1:8720 --data-dir ~/.memfuse/data

# Install SDK (in Agent project)
cd sdk && npm install && npm run build
node bin/memfuse-setup.cjs install --platform=claude-code
```

### 8.2 Data Directory Structure

```
{workspace_root}/                ← MEMFUSE_WORKSPACE_ROOT
  ├── _system/
  │   ├── metadata.sqlite        ← Resource registry, facts, cursors, etc.
  │   └── semantic.sqlite        ← Tiered FTS5 + vec0 index (high-level / detail split)
  └── tenants/{account}/{user}/  ← mfs:// URI physical root
      ├── resources/             ← Connector projections (localfs, git)
      ├── user/memories/         ← User memory files (profile, preferences, entities, events)
      ├── agent/{id}/memories/   ← Agent memory files (patterns, skills, cases, tools)
      └── session/               ← Session archives + messages.jsonl
```

### 8.3 No Docker Required

The server is designed to run as a native binary on macOS, Linux, and Windows. Docker is an optional convenience for CI/testing, not a deployment requirement.

The `docker-compose.yml` runs the single `memfuse-server` container with embedded SQLite — no PostgreSQL, no separate worker process.

## 9. Data Modeling Principles

### 9.1 Layered Truth

- `conversation_turns` is the source of truth — append-only, never modified
- `episode_chunks`, `fact_assertions`, `facts`, `memory_briefs` are derived layers — all can be rebuilt from turns
- `memory_consolidation_cursors` defines the boundary between active memory and pending overlay

### 9.2 Fact Conflict Resolution

| Value Type | Rule | Example |
|-----------|------|---------|
| Scalar | Only one `active` per `user + predicate` at a time; new value supersedes old | `location.current_city` |
| Set | Multiple `active` values allowed; new values append | `language.spoken` |
| Temporal | Time-conditioned; `valid_from`/`valid_to` determine current state; `valid_to` is populated (= now()) on supersede or retract, so only `valid_to IS NULL` rows are current | `diet.spicy_preference` |

State transitions: `active` → `superseded` (replaced by newer assertion) or `retracted` (explicitly withdrawn).

### 9.3 Episode Chunking Rules

A new episode is created when:
1. Time gap between adjacent turns > 15 minutes
2. Cumulative token count of current episode > 1200
3. (Optional) Topic drift detected

### 9.4 Data Lifecycle

```
1. Turn written synchronously (before model call)
2. Overlay: cursor-to-latest turns visible immediately as "Recent Updates"
3. Consolidation (async, triggered by session commit):
   turns → episodes → fact_assertions → facts → briefs → cursor advance
4. Recall: resolve_context embeds query → vector search → rerank → inject
5. Reinforcement: recall_count + last_recalled_at updated on each recall
6. Decay: time-based salience decay; low-value episodes archived
7. Periodic dream consolidation: background process runs every MEMFUSE_DREAM_POLL_SECS (default 300s),
   triggers consolidation when >= MEMFUSE_DREAM_MIN_SESSIONS committed sessions have accumulated
   and >= MEMFUSE_DREAM_MIN_HOURS (default 24h) since last run
```

### 9.5 Identifier Conventions

- `thread_id` = `conversation_sessions.session_id` (same concept, two names)
- `scope_id` = `thread_id` (when `scope_type=thread`) or `resource_id` (when `scope_type=resource`)
- `job scope_type` allows `thread / resource / user`
- `cursor scope_type` allows `thread / resource` only

### 9.6 Semantic Index — Tiered Table Split

`semantic.sqlite` uses a dual-table architecture to separate high-level summaries from detail-level content, enabling future physical split (cloud vs local).

| Table Pair | Levels | Content | Future Location |
|-----------|--------|---------|----------------|
| `semantic_docs_high` + `semantic_docs_high_fts` | L0 (abstract), L1 (overview) | Stable summaries, low update frequency | Cloud-eligible |
| `semantic_docs_detail` + `semantic_docs_detail_fts` | L2+ (full content, AST) | High-churn content tied to source code | Local-only |

The boundary is defined by `HIGH_LEVEL_MAX = 1` in `mfs-index`. Documents with `level <= 1` route to the high table; `level >= 2` route to the detail table.

```
semantic.sqlite
├── semantic_docs_high          ← L0/L1 main table (uri, context_type, level, title, body, embedding)
├── semantic_docs_high_fts      ← FTS5 virtual table over high
├── semantic_docs_detail        ← L2+ main table (same schema)
└── semantic_docs_detail_fts    ← FTS5 virtual table over detail
```

Search operations automatically query one or both table pairs based on level filters and merge results in Rust. The public `SearchIndex` API is unchanged — the split is transparent to callers.

On first startup with an existing single-table schema (`semantic_documents`), data is automatically migrated to the new dual-table layout.

---

## 10. CLI Architecture

CLI + Skill 模式与 MCP 共存，作为 coding agent 的主要交互界面，显著降低 token 开销。110 CLI 命令覆盖全部 43 MCP 工具和 110+ HTTP 端点。

### 10.1 Motivation

当前 MCP 界面存在 token 效率问题：

| 问题 | 具体影响 |
|------|----------|
| MCP tool schema 加载 | 43 个工具的 JSON Schema 在每次请求中注入 context，占用 ~800-1200 tokens |
| MCP 输出格式冗余 | emoji markers、footer tips、重复 `##` headers 约占输出 30-40% |
| MCP 持久连接开销 | stdio JSON-RPC 连接维持、心跳检测、孤儿进程保护 |
| Schema 变更风险 | 工具名/输入 schema 的任何改动都会破坏已有 agent session |

采用 CLI + Skill 模式后的预期收益：

| 维度 | MCP 模式 | CLI 模式 | 增益 |
|------|----------|----------|------|
| Schema 加载 | ~800-1200 tokens/request | 0（SKILL.md 仅加载一次） | **~80-90% reduction** |
| 输出格式 | verbose markdown + emoji + tips | compact markdown | **~30-40% reduction per call** |
| 连接模型 | stdio JSON-RPC 持久连接 | HTTP 请求，即连即断 | 无进程生命周期管理负担 |
| 独立安装 | 需要 MCP host 支持 | `npm install -g` 即可 | 更广泛的可用性 |

### 10.2 Client-Daemon Model

```
┌─── Daemon（持久进程）──────────────────────────────────────────────┐
│                                                                      │
│  MemFuse HTTP Server (mfs-server)                                   │
│  127.0.0.1:8720 (MEMFUSE_SERVER_URL)                               │
│  持有全部 runtime state:                                             │
│    SQLite metadata + semantic store                                 │
│    Workspace projection                                             │
│    SessionEngine + MemoryEngine                                     │
│    RetrievalEngine + WatchDaemon                                    │
│                                                                      │
│  REST API: 详见 §3.2 HTTP API Paths                                │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘

┌─── Client（每次调用独立进程）──────────────────────────────────────┐
│                                                                      │
│  memfuse <command> [options]                                         │
│  Node.js 进程 → parse args → HTTP request → format output → exit   │
│  启动时间: <200ms（无 Rust runtime 初始化）                          │
│  内存占用: <30MB                                                     │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘

┌─── Skill（静态指令文档）──────────────────────────────────────────┐
│                                                                      │
│  SKILL.md (allowed-tools: Bash(memfuse:*))                          │
│  LLM 在 session start 时读取一次，后续按文档指引调用 CLI 命令       │
│  替代 MCP tool schema 的动态注入机制                                 │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
```

**关键架构决策**：
- Daemon 即现有 `mfs-server` HTTP 服务，**不新增任何守护进程代码**
- Client 通过 `MEMFUSE_SERVER_URL` 发现 Daemon，与 MCP hooks 使用完全相同的发现机制
- 每次 CLI 调用是一次完整的 HTTP request-response-cycle，无状态残留

**与现有接口的共存**：

```
                    ┌─── SDK Package (@percena/memfuse) ──────────────┐
                    │                                          │
  MCP mode ────────│─► src/mcp/server.ts (保留，向后兼容)     │
  (现有)           │─► src/hooks/ (保留，hooks 仍用 HTTP)     │
                    │                                          │
  CLI mode ────────│─► bin/memfuse.js (新增)                  │──► HTTP Server
  (新增)           │─► src/cli/ (新增)                        │    (共享后端)
                    │                                          │
                    │─► src/shared/http.ts (共享 HTTP client) │
                    │─► src/shared/config.ts (共享配置)       │
                    └──────────────────────────────────────────┘
```

MCP 和 CLI 共享同一个 HTTP server 后端。用户可以选择 MCP 模式（持久连接）或 CLI 模式（即连即断）。

### 10.3 Command Surface

110 CLI commands map to 43 MCP tools and 110+ HTTP endpoints. See `sdk/src/cli/commands/` for full mapping.

**全局选项**：

| 选项 | 说明 | 默认值 |
|------|------|--------|
| `--json` | 输出原始 JSON | default 模式 |
| `--verbose` | 输出完整格式（含 emoji/tips） | default 模式 |
| `--server <url>` | MemFuse server URL | `MEMFUSE_SERVER_URL` 或 `http://127.0.0.1:8720` |
| `--user <id>` | user ID | `MEMFUSE_USER_ID` 或 `$USER` 或 `default` |
| `--session <id>` | session ID | `MEMFUSE_SESSION_ID` 或 `default` |
| `--api-key <key>` | API key | `MEMFUSE_API_KEY` 或空 |
| `--strategy <name>` | 搜索策略预设 | precision |
| `--at-time <timestamp>` | 点时查询（ISO 8601） | 当前时间 |

**优先级规则**：命令行选项 > 环境变量 > 默认值。

**命令命名原则**：
1. **Flat 结构**：无子命令分组，直接 `memfuse search`（减少 LLM 输入 token）
2. **kebab-case**：多词命令用 `-` 连接（`resolve-context`）
3. **动词优先**：动作类用动词（`add-resource`），查询类用名词（`list-facts`）

**CLI Flag → API 字段映射**（非直观映射记录）：

| CLI Flag | HTTP API 字段 | 说明 |
|----------|---------------|------|
| `--budget` | `token_budget` | 非直观映射 |
| `--type` | `tool_name` | store-observation |
| `--metadata` | `metadata` | JSON string → parsed object |
| `--new-stage` | `new_stage` | promote-rule 生命周期 |
| `--file` | `markdown` | import 从文件读取 |
| `--episode-ids` | `episode_ids` | 逗号分隔 → JSON array |
| `--fact-ids` | `fact_ids` | 逗号分隔 → JSON array |
| `--rule-id` | `tags[]` | create-instance → `rule:<id>` tag |
| `--value-type` | `value_type` | create-fact value type |
| positional `<from>` `<to>` | `from_uri`, `to_uri` | mv 命令 |

**多值参数规范**：数组参数统一使用逗号分隔（如 `--episode-ids ep_1,ep_2,ep_3`）。如需传入含逗号的值，改用 `--json` 模式。

### 10.4 Output Format

**三层输出模式**：

| 模式 | 激活方式 | 适用场景 |
|------|----------|----------|
| **default** | (默认) | 紧凑 markdown，无 emoji/tips — LLM 日常交互 |
| **json** | `--json` | 原始 JSON — 程序化使用、管道操作 |
| **verbose** | `--verbose` | 完整 MCP 同等格式 — 人类阅读、调试 |

**Default 模式格式化规则**：

| 规则 | MCP 格式 | CLI default 格式 |
|------|----------|-----------------|
| 区段标题 | `## Active Facts` | `Facts:` |
| 事实条目 | `- ✓ **subject** → pred: val (confidence: 0.95)` | `subject → pred: val (0.95)` |
| Episode 条目 | `- **[id]** summary (score: 0.82)` | `id summary (0.82)` |
| Heuristic 条目 | `- ★ **rule** [tags]` | `[confirmed] rule [tags]` |
| Footer tips | `*Use get_observations with...*` | (删除) |

**JSON 模式**：直接透传 server 返回的 JSON，保留所有字段。成功 exit 0，失败 exit 1。

**错误输出**：

| 场景 | 输出 | 退出码 |
|------|------|--------|
| Server 不可达 | `Error: MemFuse server not reachable at <url>. Is the server running?` | 2 |
| HTTP 4xx/5xx | `Error: Server returned <status> for <method> <path>` | 1 |
| 参数缺失 | `Error: <command> requires <param>` | 1 |

CLI 始终以非零退出码表示失败。错误通过 stderr，成功通过 stdout。

### 10.5 SKILL.md Specification

**Frontmatter**：

```yaml
---
name: memfuse
description: Persistent memory and knowledge management for coding agents. LOOK → DIG → SAVE workflow via CLI.
allowed-tools: Bash(memfuse:*)
---
```

`allowed-tools: Bash(memfuse:*)` 约束 LLM 只通过 shell 命令操作 MemFuse，绕过 MCP tool schema 加载。

**内容结构**：

```
1. Quick start (3-5 步骤)
2. LOOK-DIG-SAVE workflow
3. Commands reference
4. Output modes
5. Session & environment
6. Examples
```

**LOOK-DIG-SAVE 映射**：

```
LOOK → memfuse resolve-context <query>    # directional signals
       memfuse heuristics-l0               # session-start confirmed rules

DIG  → memfuse ls / abstract / overview / read  # 深挖 signal 指向的 URI
       memfuse search / grep / glob               # 搜索定位

SAVE → memfuse store-observation <content> --type <type>  # 存储观察
       memfuse commit-session                             # 提交触发 consolidation
       memfuse cite-memories                              # 标记有用记忆
```

### 10.6 Package Structure & Installation

CLI 作为 `@percena/memfuse` 包的一部分发布：

```
sdk/
  bin/
    memfuse.cjs              # CLI 入口
    memfuse-mcp.cjs          # MCP stdio 入口
    memfuse-setup.cjs        # 安装工具入口
  src/cli/
    index.ts                 # 命令路由 + dispatch
    types.ts                 # CliArgs, helpers
    output.ts                # 格式化工具（3 模式）
    commands/
      core.ts                # resolve-context, search, store-observation, commit-session, list-facts
      dig.ts                 # ls, read, abstract, overview, glob, grep, find
      canvas.ts              # canvas 查询与管理
      resources.ts           # add-resource, add-repo, add-inline, resources-list
      resources-extended.ts  # resource-export, resource-import, snapshots
      session.ts             # session-create, session-list, session-get, session-delete
      memory.ts              # cite-memories, export-memories, import-memories, consolidate
      facts.ts               # create-fact, supersede-fact, retract-fact, trace-fact
      heuristics.ts          # simulate-reaction, heuristics-l0, confirm-rule
      heuristics-extended.ts # create-rule, list-rules, promote-rule, create-instance
      workspace.ts           # mkdir, write, mv, rm, tree, stat, rebuild
      relations.ts           # link, unlink, relations
      watches.ts             # watches-list, resource-watch, watch-daemon-*
      skills.ts              # skills-list, add-skill
      system.ts              # system-status, health, ready, metrics, install
      code_symbols.ts        # code-symbols-list, code-symbols-search
  src/shared/
    http.ts                  # 共享 HTTP client
    config.ts                # 共享配置
    router.ts                # Backend routing
    utils.ts                 # 通用工具
```

**install 命令**：

```bash
memfuse install --skills --no-mcp          # 仅安装 Skill（推荐 CLI 模式）
memfuse install --skills --hooks --no-mcp  # Skill + Hooks（hooks 通过 HTTP 直连）
memfuse install --skills --hooks           # Skill + Hooks + MCP（向后兼容）
```

### 10.7 Non-Functional Requirements

**NFR-CLI-1: 启动速度** — CLI 进程从 argv 解析到 HTTP request 发出 < 200ms。不引入 heavy CLI framework，直接解析 `process.argv`。

**NFR-CLI-2: Token 效率** — Default 模式输出比 MCP 同等输出减少 >= 30% tokens。JSON 模式仅输出 server 原始响应。

**NFR-CLI-3: 向后兼容** — CLI 命令覆盖全部 MCP 工具功能面。MCP 模式继续可用。Hook lifecycle 不受影响。

**NFR-CLI-4: 独立安装友好** — CLI 代码与 MCP server 代码物理分离（`src/cli/` vs `src/mcp/`）。未来可拆分为独立 npm 包。约束：CLI 代码不得 import `src/mcp/` 中的任何模块。

**NFR-CLI-5: 错误处理** — 网络错误必须有清晰的人类可读消息。Server 不可达时给出启动提示。所有错误通过 stderr。

**NFR-CLI-6: 跨平台** — 支持 macOS + Linux。Node >= 18.0.0。Windows 支持为后续阶段目标。

### 10.8 Risks

| 风险 | 影响 | 缓解措施 |
|------|------|----------|
| CLI 每次调用是新进程，比 MCP 持久连接慢 | 每次调用 ~200ms 进程启动 | HTTP 请求本身 < 50ms，总延迟可接受 |
| LLM 可能不遵守 `allowed-tools` 约束 | LLM 可能仍尝试调用 MCP | MCP 模式保留向后兼容；SKILL.md 明确强调 CLI 是首选方式 |
