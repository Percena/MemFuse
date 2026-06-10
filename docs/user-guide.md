# MemFuse 用户指南

> **信号灯塔**：MemFuse 告诉 Agent **相关信息在哪里** 和 **是什么类型**，而不是直接给出精确答案。Agent 收到方向信号后，主动去读取原文细节。

---

## 1. 快速入门

MemFuse 是一个为 AI 编程 Agent（Claude Code、Codex）提供跨会话持久化记忆的系统。Rust 单进程 + 嵌入式 SQLite，无外部依赖。

### 前置要求

- **Rust 1.85+** — 构建 server
- **Node.js 18+** — SDK / CLI / MCP server / Hooks
- **SQLite** — 嵌入式，无需额外安装

### 启动 Server

```bash
# 开发模式：构建并运行
cargo build --release -p mfs-server
./run-server.sh

# 独立二进制：不依赖 repo .env，默认使用 OS 用户配置/数据目录
./target/release/mfs-server --print-config
./target/release/mfs-server
```

源码仓库的 `./run-server.sh` 默认监听 **18720** 端口：

```bash
curl http://127.0.0.1:18720/health
# → {"status":"alive","version":"0.1.0","summary_provider":"openai","embedding_provider":"jina"}
```

独立二进制、系统服务与源码仓库现统一使用内置默认端口 `18720`（唯一事实来源：Rust `mfs-types`、TypeScript `sdk/src/shared/config.ts`）。运行时通过 `.env` / 环境变量中的 `MEMFUSE_BIND_ADDR`、`MEMFUSE_SERVER_URL` 覆盖，无需改源码。

**开发环境变量（`.env` 或 `.env.example`）：**

| 变量 | 默认值 | 说明 |
|------|--------|------|
| `MEMFUSE_WORKSPACE_ROOT` | OS 用户数据目录 | 数据存储根目录；repo 开发时可显式指定 |
| `MEMFUSE_BIND_ADDR` | `127.0.0.1:18720` | repo 开发绑定地址 |
| `MEMFUSE_OPENAI_API_KEY` | — | 启用 LLM 提取（`OPENAI_API_KEY` 也可作为 fallback） |
| `MEMFUSE_OPENAI_API_BASE` | `https://api.openai.com/v1` | 兼容 LLM 端点（`OPENAI_BASE_URL` 也可作为 fallback） |
| `MEMFUSE_JINA_API_KEY` | — | 启用语义搜索（可选） |
| `RUST_LOG` | `info` | 日志级别（标准 Rust 日志控制） |
| `MEMFUSE_DREAM_MIN_HOURS` | `24` | 定期合并：最小间隔小时数 |
| `MEMFUSE_DREAM_MIN_SESSIONS` | `5` | 定期合并：最小已提交会话数 |
| `MEMFUSE_DREAM_POLL_SECS` | `300` | 定期合并：轮询间隔秒数 |

> **Provider 默认策略**：系统会自动检测 LLM provider。有 `MEMFUSE_OPENAI_API_KEY` / `OPENAI_API_KEY` 时，summary/chat 默认使用 OpenAI-compatible LLM；有 `MEMFUSE_JINA_API_KEY` 时，embedding/rerank 默认使用 Jina。无 LLM Key 时自动降级为 **deterministic 模式**（regex/keyword/Jaccard），功能完整，不会阻塞。

完整环境变量列表见 `.env.example`。

### 构建 SDK

```bash
cd sdk && npm install && npm run build
```

构建后 `memfuse` CLI 命令可用（110 命令，100% HTTP API 覆盖）：

```bash
MEMFUSE_SERVER_URL=http://127.0.0.1:18720 node bin/memfuse.cjs health
# → Server online (http://127.0.0.1:18720)
```

### 安装到 Agent 平台

```bash
# Claude Code（8 hooks + 110 CLI commands + 1 skill + MCP）
node bin/memfuse-setup.cjs install --platform=claude-code --server-url=http://127.0.0.1:18720

# Codex（3 hooks + 110 CLI commands + 1 skill + MCP）
node bin/memfuse-setup.cjs install --platform=codex --server-url=http://127.0.0.1:18720
```

安装后 Agent 自动获得：

| 集成层 | Claude Code | Codex | 说明 |
|--------|-------------|-------|------|
| **CLI** (`memfuse`) | 110 命令 | 110 命令 | Agent 主路径，100% HTTP API 覆盖 |
| **Skill** | `.claude/skills/memfuse/SKILL.md` | `.codex/skills/memfuse/SKILL.md` | CLI-oriented，`allowed-tools: Bash(memfuse:*)` |
| Hooks | 8 个 | 3 个 | 生命周期自动触发 |
| MCP | 43 tools | 43 tools | 可选便利层（~25% 覆盖） |
| Plugin Manifest | `.claude-plugin/plugin.json` | `.codex-plugin/plugin.json` | 平台原生插件发现 |

> **推荐使用 CLI 作为 Agent 交互路径**。MCP 仅覆盖 ~25% API，无法完成 session/facts/memory 管道的完整闭环。

### Docker（可选）

```bash
docker compose up --build
```

---

## 2. 交互路径

MemFuse 提供 **两条交互路径**，推荐优先使用 CLI：

| 路径 | 命令/工具 | 覆盖率 | 适用场景 |
|------|----------|--------|----------|
| **CLI（推荐）** | `memfuse <command>` | **100%** (110 命令 / 110+ 路由) | Agent 主路径，token 效率最优 |
| MCP | MCP tools (43 个) | ~25% | MCP 原生平台便利层，仅覆盖核心读取操作 |

> **CLI 和 MCP 共享同一个 HTTP server 后端**，功能等价但覆盖度不同。CLI 覆盖全部 HTTP API 路由，MCP 仅覆盖核心读取操作。
> 详见 [architecture.md](architecture.md)。

### 两套 CLI 的区别

| CLI | 二进制名 | 覆盖 | 适用场景 |
|-----|---------|------|----------|
| **SDK CLI** (`memfuse`) | `memfuse` | 110 命令，100% HTTP API 覆盖 | **Agent 主路径** — session/facts/memory/heuristics 全覆盖 |
| Rust CLI (`mfs-cli`) | `mfs-cli` | ~50 命令，离线操作 | 调试/运维 — 仅查询/资源管理，缺 session 创建/facts 写入/memory 管道 |

> Agent 集成应使用 `memfuse`（SDK CLI），不要使用 `mfs-cli`（Rust CLI）。

---

## 3. 核心工作流：LOOK → DIG → SAVE

### CLI 模式（推荐）

| 步骤 | 操作 | CLI 命令 | Token 消耗 |
|------|------|---------|------------|
| **LOOK** | 拉取方向信号 | `memfuse resolve-context "task description"` | ~500-1500 |
| **DIG** | 渐进式深挖 | `memfuse search` → `memfuse timeline` → `memfuse get-observations` | 渐进增长 |
| **SAVE** | 主动存储决策 | `memfuse store-observation "..." --type Decision` | 极低 |

**关键原则**：Hooks 是被动的，Skills 是主动的。必须主动调用 `memfuse resolve-context` 和 `memfuse store-observation`。

### MCP 模式（向后兼容参考）

| 步骤 | 操作 | MCP Tool | Token 消耗 |
|------|------|----------|------------|
| **LOOK** | 拉取方向信号 | `resolve_context(query)` | ~500-1500 |
| **DIG** | 渐进式深挖 | `search_memories` → `timeline` → `get_observations` | 渐进增长 |
| **SAVE** | 主动存储决策 | `store_observation(tool_name, content)` | 极低 |

### 渐进式检索（节省 Token）

始终从紧凑层开始，只对高相关结果深挖：

| 层级 | CLI 命令 | MCP Tool | Token 消耗 | 用途 |
|------|---------|----------|------------|------|
| 1 | `memfuse search` | `search_memories` | 50-100/条 | 紧凑索引 |
| 2 | `memfuse timeline <id>` | `timeline` | 200-400 | 时间线上下文 |
| 3 | `memfuse get-observations <ids>` | `get_observations` | 500-1000/条 | 完整详情 |

---

## 4. CLI Commands（110 命令）

### Core (LOOK-DIG-SAVE)

| Command | Purpose |
|---------|---------|
| `memfuse resolve-context <query>` | Pull directional memory signals (--budget N, --strategy, --at-time). `inject-context` is an alias. |
| `memfuse search <query>` | Search episodic memories (--limit, --strategy) |
| `memfuse store-observation [text]` | Save observation (--type, --input, --output) |
| `memfuse commit-session` | Trigger memory consolidation (--reason) |
| `memfuse list-facts` | List active facts |

### DIG (inspect resources)

| Command | Purpose |
|---------|---------|
| `memfuse ls <uri>` | List directory entries |
| `memfuse read <uri>` | Read full content |
| `memfuse abstract <uri>` | L0 summary |
| `memfuse overview <uri>` | L1 overview |
| `memfuse glob <uri> <pattern>` | Glob match files |
| `memfuse grep <query>` | Keyword search (--target, --limit) |
| `memfuse find <query>` | Path-based search (--target) |
| `memfuse search-context <query>` | Context-aware search (--session-context) |

### Memory Management

| Command | Purpose |
|---------|---------|
| `memfuse create-fact <subj> <pred> <val>` | Create a new fact (--confidence) |
| `memfuse supersede-fact <id>` | Supersede an old fact |
| `memfuse retract-fact <id>` | Retract an incorrect fact (--reason) |
| `memfuse trace-fact <id>` | Trace fact provenance to source episodes |
| `memfuse timeline <episode-id>` | Chronological context (--direction, --radius) |
| `memfuse get-observations <ids>` | Full episode details (comma-separated IDs) |
| `memfuse cite-memories` | Mark useful memories (--episode-ids, --fact-ids) |
| `memfuse export-memories` | Export as Markdown |
| `memfuse import-memories --file <path>` | Import from Markdown (or stdin) |

### Resources

| Command | Purpose |
|---------|---------|
| `memfuse add-resource` | Add resource (--source-kind, --source-path, --url) |
| `memfuse add-repo <path\|url>` | Add git repo (--logical-name, --branch) |
| `memfuse add-inline <name> <text>` | Add inline content (--logical-name) |
| `memfuse resources-list` | List all resources |
| `memfuse resource-refresh <id>` | Refresh a resource |
| `memfuse resource-rebuild <id>` | Rebuild a resource |
| `memfuse resource-export <id>` | Export resource pack |
| `memfuse resource-import <path>` | Import resource pack (--name) |
| `memfuse task-status <task-key>` | Check background task status |
| `memfuse tasks-list` | List recent tasks (--limit) |

### Heuristics & Skills

| Command | Purpose |
|---------|---------|
| `memfuse simulate-reaction <scenario>` | Predict user reaction (--tags) |
| `memfuse heuristics-l0` | Top confirmed rules (--max-rules) |
| `memfuse confirm-rule <rule-id>` | Mark rule as confirmed |
| `memfuse create-rule <text>` | Create heuristic rule (--tags) |
| `memfuse list-rules` | List rules (--lifecycle-stage) |
| `memfuse get-rule <rule-id>` | Get rule detail |
| `memfuse promote-rule <rule-id>` | Promote rule lifecycle stage |
| `memfuse create-instance <rule-id>` | Create heuristic instance (--context) |
| `memfuse list-instances` | List instances (--rule-id) |
| `memfuse retrieve <intent>` | Retrieve matching heuristics (--tags, --top-k) |
| `memfuse skills-list` | List registered skills |
| `memfuse add-skill <path>` | Ingest a skill |

### Session Lifecycle

| Command | Purpose |
|---------|---------|
| `memfuse session-create` | Create session (--session <id>) |
| `memfuse session-list` | List sessions |
| `memfuse session-get <id>` | Get session detail |
| `memfuse session-context <id>` | Assembled context (--token-budget) |
| `memfuse add-message <sid> <content>` | Add message (--role user/assistant) |
| `memfuse commit-session` | Trigger memory consolidation (--reason) |
| `memfuse session-delete <id>` | Delete session |

### Workspace / Relations / Watches / Code Symbols / System

| Command | Purpose |
|---------|---------|
| `memfuse mkdir/write/mv/rm` | Workspace write operations |
| `memfuse link/unlink/relations` | URI relation management |
| `memfuse watches-list/resource-watch/watch-daemon-*` | Watch management |
| `memfuse code-symbols-list/search/create/delete` | Code symbol views |
| `memfuse health/system-status/observer-status` | System status |
| `memfuse install` | Deploy skills/hooks/MCP |

**Full 110-command reference**: see the installed SKILL.md at `.claude/skills/memfuse/SKILL.md` or `.codex/skills/memfuse/SKILL.md`.

### Output Modes

| Mode | Flag | Tokens | Use when |
|------|------|--------|----------|
| **Default** | (none) | ~30-40% of MCP | Normal work — compact |
| **JSON** | `--json` | Raw response | Scripts/pipelines |
| **Verbose** | `--verbose` | MCP-equivalent | Full detail |

### Search Strategies (--strategy)

| Strategy | Behavior | When to use |
|----------|----------|------------|
| `precision` | Pure relevance (default) | Normal queries |
| `diverse` | Relevance + MMR diversity | "All related topics" |
| `recent` | Stronger recency boost | "Recent decisions" |
| `comprehensive` | Budget ×2 + lower threshold | "Everything about X" |

---

## 5. MCP Tools（43 个）— 向后兼容参考

> MCP tools 仅覆盖 ~25% 的 HTTP API。推荐优先使用 CLI 命令（110 命令，100% 覆盖）。
> MCP 是可选便利层，不是 Agent 标准对接路径。

### Context & Search

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `resolve_context` | `resolve-context` | 一站式上下文注入 | `POST /context/resolve` |
| `inject_context` | `resolve-context` (alias) | 含行为启发式的上下文注入 | `POST /context/resolve` |
| `search_memories` | `search` | 搜索记忆 | `POST /v1/memory:search` |
| `timeline` | `timeline` | Episode 时间线 | `GET /episodes/{id}/timeline` |
| `get_observations` | `get-observations` | Episode 详情 | `GET /episodes/{id}` |
| `list_facts` | `list-facts` | 列出活跃事实 | `GET /facts` |
| `facts_at_time` | `resolve-context --at-time` | 时间点事实查询 | `POST /context/resolve` |
| `trace_fact` | `trace-fact` | Fact 溯源 | `GET /facts/{id}/trace` |

### Session & Observation

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `store_observation` | `store-observation` | 存储观察 | `POST /sessions/{id}/observations` |
| `commit_session` | `commit-session` | 提交会话触发巩固 | `POST /sessions/{id}/commit` |
| `session_create` | `session-create` | 创建会话 | `POST /sessions` |
| `session_list` | `session-list` | 列出会话 | `GET /sessions` |
| `session_get` | `session-get` | 获取会话详情 | `GET /sessions/{id}` |
| `session_delete` | `session-delete` | 删除会话 | `DELETE /sessions/{id}` |
| `add_message` | `add-message` | 添加消息 | `POST /sessions/{id}/messages` |

### Fact Management

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `create_fact` | `create-fact` | 创建事实 | `POST /facts` |
| `supersede_fact` | `supersede-fact` | 取代旧事实 | `POST /facts/{id}/supersede` |
| `retract_fact` | `retract-fact` | 撤回事实 | `POST /facts/{id}/retract` |

### Memory Operations

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `cite_memories` | `cite-memories` | 记录有用的 episode/fact | `POST /memories/cite` |
| `export_memories` | `export-memories` | 导出记忆为 Markdown | `GET /memories/export` |
| `import_memories` | `import-memories` | 从 Markdown 导入记忆 | `POST /memories/import` |
| `consolidate` | `consolidate` | 触发记忆巩固 | `POST /v1/memory:consolidate` |
| `extract_facts` | `extract-facts` | 触发事实提取 | `POST /v1/memory:extract-facts` |

### Resource & DIG

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `add_resource` | `add-resource` | 注册 localfs 资源 | `POST /resources` |
| `add_repo` | `add-repo` | 添加 Git 仓库 | `POST /resources` |
| `add_resource_inline` | `add-inline` | 创建内联资源 | `POST /resources` |
| `task_status` | `task-status` | 检查后台任务状态 | `GET /tasks/{key}` |
| `ls` | `ls` | 资源目录列表 | `GET /ls` |
| `read` | `read` | 全文读取 | `GET /read` |
| `abstract` | `abstract` | L0 摘要 | `GET /abstract` |
| `overview` | `overview` | L1 概览 | `GET /overview` |
| `glob` | `glob` | 文件模式匹配 | `GET /glob` |
| `grep` | `grep` | 关键词搜索 | `GET /grep` |

### Heuristics

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `simulate_reaction` | `simulate-reaction` | 模拟用户反应 | `POST /heuristics/simulate-reaction` |
| `heuristics_l0_confirmed` | `heuristics-l0` | 已确认的启发式规则 | `POST /heuristics/l0-confirmed` |
| `heuristics_confirm_rule` | `confirm-rule` | 标记规则为用户确认 | `POST /heuristics/rules/{id}/confirm` |

### Relations & Canvas

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `link_relations` | `link` | 创建 URI 关系 | `POST /relations` |
| `list_relations` | `relations` | 列出 URI 关系 | `GET /relations` |
| `get_repo_manifest` | — | 获取仓库 manifest | `GET /manifest/get` |
| `query_canvas` | — | 查询 Canvas 结构 | `GET /canvas/query` |
| `propose_active_overlay` | — | 提议 Active Overlay | `POST /overlay/propose` |
| `report_conflict` | — | 报告 Overlay 冲突 | `POST /overlay/report_conflict` |

### Guide

| Tool | CLI 对应 | 用途 | 对应端点 |
|------|---------|------|----------|
| `memfuse_guide` | (内置) | 使用指南（MCP 模式） | 内置静态文本 |

---

## 6. 观察类型

> 以下为推荐使用的观察分类约定。`store-observation --type` 接受任意字符串，不做枚举校验。

| 类型 | 适用场景 | 示例 |
|------|----------|------|
| `Decision` | 有意识的决策 | "Decided async consolidation over batch" |
| `Discovery` | 意外发现 | "Test suite has hidden env var dependency" |
| `BugFix` | 修复 bug（含根因） | "Fixed race condition in episode dedup" |
| `Change` | 影响后续工作的变更 | "API endpoint /context/resolve now returns markers" |
| `Feature` | 新增能力 | "Added LLM-assisted fact extraction" |
| `Refactor` | 结构性重构 | "Moved ChatProvider usage to mfs-memory" |
| `Gotcha` | 易错点 | "SQLite WAL mode thread management in async" |
| `Pattern` | 可复用的模式 | "LLM first + deterministic fallback pattern" |
| `ManualNote` | 其他重要事实 | "User prefers snake_case for Rust" |

---

## 7. 资源导入

### CLI 模式

```bash
# 导入文档目录（localfs）
memfuse add-resource --source-kind localfs --source-path /path/to/docs --logical-name my-docs

# 导入 Git 仓库
memfuse add-repo /path/to/repo --logical-name my-project
memfuse add-repo https://github.com/org/repo --logical-name remote-project

# 添加内联内容
memfuse add-inline notes.md "content here..." --logical-name inline-notes

# 等待导入完成
memfuse task-status <task-key>
memfuse wait-task <task-key> --timeout-ms 10000

# 导入后操作
memfuse resource-refresh <resource-id>
memfuse resource-rebuild <resource-id>
```

### HTTP API 模式

```bash
# 导入文档目录（localfs）
curl -X POST -H "Content-Type: application/json" \
  -d '{"source_kind":"localfs","source_path":"/path/to/docs","logical_name":"my-docs"}' \
  http://127.0.0.1:18720/resources

# 导入 Git 仓库
curl -X POST -H "Content-Type: application/json" \
  -d '{"source_kind":"git","source_path":"/path/to/repo","logical_name":"my-project"}' \
  http://127.0.0.1:18720/resources

# 等待导入完成
curl "http://127.0.0.1:18720/tasks/{task_key}/wait"
```

### URI 格式

| source_kind | URI 格式 |
|-------------|----------|
| `localfs` | `mfs://resources/localfs/{name}` |
| `git` | `mfs://resources/git/{host}/{namespace}/{repo}` |
| `inline` | `mfs://resources/inline/{name}` |

### Family Detection

同一 Git 仓库（相同 `host + namespace + repo`）重复注册时，自动刷新已有资源而非创建新资源，保留已有 episode/facts 数据。

---

## 8. 数据存储

### 目录结构

```
{workspace_root}/
├── _system/
│   ├── metadata.sqlite     ← 主数据库（~30 张表）
│   └── semantic.sqlite     ← 语义索引（向量搜索 + FTS5）
└── tenants/{account}/{user}/
    ├── resources/          ← 资源投影文件（localfs/git）
    ├── user/memories/      ← 用户记忆（profile, preferences, entities, events）
    ├── agent/{id}/memories/ ← Agent 记忆（patterns, skills, cases, tools）
    └── session/            ← 会话数据
```

### 查看数据

**CLI（推荐）**：

```bash
memfuse list-facts                     # 活跃事实
memfuse resolve-context "preferences"  # 上下文注入
memfuse session-list                   # 会话列表
memfuse get-observations <episode-id>  # Episode 详情
memfuse timeline <episode-id>          # Episode 时间线
memfuse health                         # 健康检查
memfuse system-status                  # 系统状态
memfuse resources-list                 # 资源列表
memfuse cite-memories --episode-ids ... --fact-ids ...
memfuse export-memories                # 导出
memfuse trace-fact <fact-id>           # Fact 溯源
```

**Rust CLI（调试/运维，仅查询）**：

```bash
mfs-cli --workspace-root /path/to/workspace resources-list
mfs-cli session-list
mfs-cli search --query "authentication"
mfs-cli system-status
mfs-cli doctor
```

**SQLite 直查**：

```bash
sqlite3 {workspace_root}/_system/metadata.sqlite

# 活跃事实
SELECT predicate, display_value, confidence, status FROM facts WHERE status = 'active';

# 所有 Episode
SELECT episode_id, summary, salience_score, recall_count FROM episode_chunks;

# 审计日志
SELECT event_type, subject_uri, recorded_at FROM audit_log ORDER BY recorded_at DESC LIMIT 20;
```

### 数据生命周期

- **Facts**：由 LLM 或 regex 从对话自动提取 → `active` → `superseded`（被新事实取代）或 `retracted`（主动撤销）。`valid_from` 在创建时填充，`valid_to` 在 supersede/retract 时填充。可通过 `memfuse trace-fact` 回溯来源 episode。
- **Episodes**：consolidation pipeline 从对话轮次自动生成 → 语义检索 → recall 强化 → 低价值归档
- **Resources**：注册 → 自动生成 L0/L1 摘要 + 路径索引 + 语义向量 → 可刷新/重建

---

## 9. LLM 辅助管线

| 模块 | LLM 操作 | Deterministic 回退 |
|------|----------|-------------------|
| **facts** | 8 类 taxonomy 提取 | 45 条 regex 规则（21 种 predicate，含7个程序性谓词） |
| **episodes** | L0/L1 摘要 + salience_hint | 截断拼接（200/500 字符） |
| **intent** | 语义意图分类 | 双语 keyword 匹配 |
| **consolidation** | 语义去重（skip/merge/replace/create） | Jaccard token overlap |

所有 LLM 调用具备 **circuit breaker** 保护，系统永不阻塞于 LLM 失败。

**Confidence 标记**：`✓` (>0.8) / `~` (0.5-0.8) / `?` (<0.5)

---

## 10. Hooks 与 Skill 集成

### Hooks（被动捕获）

| Hook | Claude Code | Codex | 触发时机 | 作用 |
|------|:-----------:|:-----:|----------|------|
| Setup | ✓ | — | 代理启动 | 健康检查 |
| SessionStart | ✓ | ✓ | 新会话开始 | 注入 resolve-context 结果 |
| PreToolUse[Read] | ✓ | — | 读取文件前 | 注入该文件的历史观察 |
| UserPromptSubmit | ✓ | — | 用户提交 prompt | 中途语义上下文注入 |
| PostToolUse | ✓ | ✓ | 工具执行后 | 自动捕获工具使用，含 source_trust 标记 |
| Stop | ✓ | ✓ | 回合结束 | 生成 SessionMemory 回合摘要 |
| PreCompact | ✓ | — | 压缩前 | 保存完整上下文 |
| SessionEnd | ✓ | — | 会话结束 | 触发 consolidation pipeline |

### Skill（主动指引）

Skill 文件：`sdk/src/skills/memfuse/SKILL.md`（CLI-oriented，含 `allowed-tools: Bash(memfuse:*)`）

安装路径：
- Claude Code：`.claude/skills/memfuse/SKILL.md`
- Codex：`.codex/skills/memfuse/SKILL.md`

更新 Skill：
```bash
cd sdk && npm run build && node bin/memfuse-setup.cjs install --platform=claude-code --server-url=http://127.0.0.1:18720
```

---

## 11. 开发与构建

```bash
# Rust server
cargo build --release -p mfs-server
cargo test

# TypeScript SDK
cd sdk && npm run build
cd sdk && node --test tests/sdk.test.mjs

# 运行 server
MEMFUSE_WORKSPACE_ROOT=/path/to/ws cargo run -p mfs-server
```

---

## 12. 卸载

```bash
node bin/memfuse-setup.cjs uninstall --platform=claude-code
```

---

## 13. 常见问题

**Server 启动失败**
- 确认 `MEMFUSE_WORKSPACE_ROOT` 已设置且目录存在
- 源码仓库的 `./run-server.sh`：确认端口 18720 未被占用
- 独立二进制/系统服务：同样默认 18720（可通过 `MEMFUSE_BIND_ADDR` 更换）

**Hooks 不触发**
- Claude Code：确认 `.claude/settings.local.json` 中包含 memfuse hooks
- Codex：确认 `config.toml` 中 `[features].hooks = true`，并确认 MemFuse hooks 已被 trust；`memfuse install --platform=codex` 会自动写入所需 trust state。

**降级模式（deterministic）**
无 LLM Key 时自动使用：45 条 regex 规则提取事实（21 种 predicate）、截断拼接生成摘要、keyword + Jaccard 匹配搜索。完整语义功能需设置 `OPENAI_API_KEY` 或 `MEMFUSE_JINA_API_KEY`。

**无语义搜索结果**
设置 `MEMFUSE_JINA_API_KEY` 或 `OPENAI_API_KEY` 以启用 embedding 生成。

**自签名证书错误**
设置 `MEMFUSE_TLS_INSECURE=1` 以跳过 HTTPS 证书验证。

---

## 文档导航

| 文档 | 内容 |
|------|------|
| [architecture.md](architecture.md) | 系统架构：crate 依赖、API 路由、数据模型、SDK 结构 |
