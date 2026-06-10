# MemFuse 全面 Review（架构 / 功能模块 / 接口设计 / 代码质量 / 平台适配）

> 日期：2026-06-10
> 范围：Rust workspace（14 crates）+ TypeScript SDK（hooks / MCP / CLI / 安装器）+ 文档 + 测试 + CI
> 方法：全量阅读 SDK 适配层与安装链路；服务端核心路径（路由注册、auth、`mfs-memory::service`、`http/memory/context.rs`）精读；其余抽查。
> 关键论断已对照 Claude Code 官方 hooks 文档（code.claude.com/docs/en/hooks）逐条复核。

---

## 1. 总体结论

**记忆模型设计和 Rust 服务端质量明显高于 SDK 适配层的工程完成度。**
Facts / Episodes / Heuristics 三类记忆 + 遗忘曲线 + consolidation cursor + L0/L1/L2 分层的设计成熟自洽，文档与代码对应度高。但"用户无感的自动 Memory CRUD"这条主线在真实 Claude Code 上**修复前实际不生效**：hooks 安装格式错误导致一个钩子都不会触发，叠加 bin 产物缺失、默认端口不一致、离线静默降级三个问题，链路在四个环节各断一次。这批问题能存活的根本原因是 e2e 测试直接向 hook 脚本 stdin 喂 JSON，绕过了 Claude Code 的真实 hook 注册路径。

### "无感 CRUD"能力评估

| 环节 | 设计 | 修复前现状（Claude Code） | 修复后 |
|------|------|--------------------------|--------|
| C：自动捕获 | PostToolUse/Stop 观察 + SessionEnd 提交 | hooks 不触发，全断 | ✅ 生效 |
| R：自动注入 | SessionStart / UserPromptSubmit / PreToolUse 三级注入 | hooks 不触发；即使触发 PreToolUse 纯文本也进不了模型上下文 | ✅ 生效 |
| U/D：演化与遗忘 | 服务端 supersede / retract / decay / archive，无需 agent 参与 | 服务端机制本身可用 | ✅ 不变 |
| Codex | 3 hooks（SessionStart/PostToolUse/Stop）+ Stop 代偿 SessionEnd | 依赖支持 hooks 的特定 Codex 构建 | ⚠️ 同左，安装时已加明确提示 |

结论：修复 P0 后，"无感 CRUD"在 Claude Code 上成立；Codex 取决于宿主是否支持 hooks（官方 CLI 不支持时自动降级为 MCP + Skill 模式，仍可用但需 agent 主动调用）。

---

## 2. 架构评估

### 亮点（应保持）

- **分层与依赖治理**：14 个 crate 依赖图清晰无环；"mfs-memory 管 what/how、storage crates 管 where"的边界划分明确（architecture.md §7 与代码一致）。
- **Thin handler + service facade**：`mfs-memory/src/service.rs` 把 resolve_context 的 10 步管线从 handler 收敛为单入口，HTTP 层只做参数解析 / IO / 审计。
- **安全实现严谨**：`auth.rs` constant-time 比较（含长度泄露防护）；`http/mod.rs::validate_path_within_workspace` 的 canonicalize-ancestor-walk 路径穿越防护处理了"目标尚不存在"的场景。
- **降级哲学正确**：无 API key 时 deterministic 模式（regex facts + 关键词检索）永不阻塞；hooks 全部 fail-open（exit 0）不阻塞 agent——问题只在失败的可观测性（见 P0-4）。
- **CLI + Skill 替代 MCP schema 注入**：token 开销论证量化、方向正确；`SKILL.md`（LOOK→DIG→SAVE、保存/不保存准则、失败行为约定、输出三模式）是高质量的 agent skill 文档。

### 主要风险

- **读路径扩展性**（遗留建议，见 §6-R1）：`service.rs::resolve_context` 把该用户全部 episodes 拉入内存、逐条 `parse_embedding_json` 算余弦相似度，绕开了 semantic.sqlite 现成的 vec0 向量索引；同一请求内逐条同步 SQLite UPDATE 写回。记忆量上千后每个 prompt 的注入延迟线性恶化。
- **每次工具调用的进程开销**（遗留建议，见 §6-R2）：PostToolUse 无 matcher，每个工具调用 spawn 一个 node 进程 + 2 个 HTTP 请求（ensureSession + observation）。

---

## 3. 问题清单与修复状态

### P0 — 阻断"无感 CRUD"

| # | 问题 | 位置 | 影响 | 状态 |
|---|------|------|------|------|
| P0-1 | Claude Code hooks 写入格式错误：写成 `hooks: [{event, cmd:[...]}]` 数组，真实 schema 是按事件名分组的对象 `hooks: { EventName: [{matcher?, hooks:[{type:"command", command, timeout}]}] }`。`uninstall.ts` 注释 "Claude Code format: array / Codex format: nested object" 证实两个平台的格式被记反（给 Codex 写的恰好是 Claude Code 的格式）。`mergeJson` 浅合并还会用数组覆盖用户已有的 hooks 对象。 | `sdk/src/setup/install.ts`、`uninstall.ts` | 真实 Claude Code 中 8 个 hooks 全部不触发；可能破坏用户已有 hooks 配置 | ✅ 已修复 |
| P0-2 | `sdk/bin/` 不存在且被 `.gitignore`，但 `package.json` bin 字段、install.ts 的 hook command、cli.test.mjs 全部指向它；`npm run build` 不生成该目录 | `sdk/.gitignore`、`sdk/package.json` | fresh clone 无法产出可用 npm 包；发布产物不可复现 | ✅ 已修复（bin wrapper 提交入库） |
| P0-3 | 默认端口散落三处不一致：SDK `config.ts`/`install.ts`/`service/manager.ts` 默认 8720，repo server / .env.example 用 18720，Docker 用 8720 | 全仓 13 个文件 | 按 README 步骤安装后 hooks/MCP 指向错误端口，叠加 P0-4 完全无法察觉 | ✅ 已修复（全局统一 18720，TS/Rust 各一处常量做唯一事实来源，运行时一律可被 `MEMFUSE_SERVER_URL`/`MEMFUSE_BIND_ADDR`（.env）覆盖） |
| P0-4 | `callBackend` 对非 canvas 路径的网络错误**返回**伪造的 `{status:'unavailable'}` 对象而不抛错：观察写入"成功"实则丢数据；SessionStart 把"服务没起"渲染成"没有记忆"；hooks 中的 `isDegradableError` 分支对网络错误成为死代码 | `sdk/src/shared/http.ts:197-204` | 写路径静默丢数据，故障不可观测 | ✅ 已修复（非 canvas 网络错误改为抛出，canvas 降级链保留；hooks 既有降级分支恢复生效） |

### P1 — 适配层功能性缺陷

| # | 问题 | 位置 | 影响 | 状态 |
|---|------|------|------|------|
| P1-1 | Stop hook 读 `last_assistant_message`，Claude Code Stop 事件提供的字段是 `assistant_message` | `sdk/src/hooks/platform-utils.ts` | TurnSummary / SessionMemory 在 Claude Code 上永远跳过 | ✅ 已修复（adaptInput 增加字段映射） |
| P1-2 | PreToolUse 对 Claude Code 输出纯文本 stdout，但官方语义中 PreToolUse 纯文本**不**注入模型上下文（仅 SessionStart/UserPromptSubmit/PostToolUse 等支持纯文本注入）；需 JSON `hookSpecificOutput.additionalContext` | `sdk/src/hooks/platform-utils.ts::formatOutput` | "读文件前的记忆灯塔"到不了模型 | ✅ 已修复（PreToolUse 输出 JSON additionalContext；刻意**不**携带 `permissionDecision`，避免绕过用户权限确认） |
| P1-3 | `detectPlatform` 启发式误判：Claude Code 的 Bash 工具 `tool_input` 恰好是 `{command}`，所有 Bash 调用被判为 Codex | `sdk/src/hooks/platform-utils.ts:49` | 平台相关行为（输出格式、字段映射）错乱 | ✅ 已修复（安装时 hook command 显式传 `--platform=<p>`，运行时优先读取；启发式仅作 fallback） |
| P1-4 | MCP 与 hooks 的 session 分裂：MCP server 用启动时 env 的 `sessionId \|\| 'default'`，hooks 用事件里的真实 session_id；两边写入不同 session，resolve_context 的 overlay 对不上 | `sdk/src/mcp/server.ts` | Agent 主动记忆与自动捕获互相不可见 | ✅ 已修复（`resolve_context`/`inject_context`/`store_observation`/`commit_session` 增加可选 `session_id` 参数） |
| P1-5 | 自动注入污染强化信号：UserPromptSubmit 每个 prompt 调 resolve_context，服务端对每条注入的 fact/episode 无差别 `+recall_count`，被动注入 ≠ 真实使用，遗忘曲线的 reinforcement 语义失真，且与 `cite_memories` 显式反馈通道矛盾 | `crates/mfs-memory/src/service.rs::writeback_recall_and_access_log` | 衰减/强化机制失效 | ✅ 已修复（请求新增 `recall_source` 字段；hooks 传 `"auto"`，服务端对 auto 跳过 recall 写回；agent 主动调用与 cite 仍计强化） |
| P1-6 | Codex hooks 依赖非官方/特定构建特性（config.toml `[features] hooks` + trusted hash），官方 Codex CLI 不支持 hooks，e2e 自己也有探测注释 | `sdk/src/setup/install.ts` | 用户以为装好了，实际 hooks 不生效 | ✅ 已修复（安装时输出明确告知：宿主不支持 hooks 时自动降级为 MCP + Skill 模式） |

### P2 — 代码质量与工程卫生

| # | 问题 | 位置 | 状态 |
|---|------|------|------|
| P2-1 | 带 `fn main()` 的草稿文件提交入库（不在 src/，不参与编译，纯死文件） | `crates/mfs-retrieval/tests_planner_scratch.rs` | ✅ 已删除 |
| P2-2 | `Cargo.lock` 被 gitignore——发布 binary 的 workspace 应提交 lockfile 保证可复现构建 | `.gitignore` | ✅ 已修复（取消 ignore 并提交 lockfile） |
| P2-3 | migrations 编号有按库分流的重复变体（0013×2、0014×3、0017×2）且缺 0004/0021，对新贡献者是陷阱 | `crates/mfs-metadata/src/migrations/` | 📝 遗留建议（在迁移目录加 README 说明编号归属，或拆分目录） |
| P2-4 | `uninstall.ts` 的格式注释（Claude Code/Codex 标反）与新格式同步 | `sdk/src/setup/uninstall.ts` | ✅ 已修复 |
| P2-5 | 文档端口表述混乱（README ASCII 图 8720、user-guide 双端口模型、sdk/README 默认值） | docs / README | ✅ 已统一为 18720 |

### 复核修正（原口头分析中的两处错误，以本文为准）

1. **`Setup` 是合法的 Claude Code hook 事件**（仅在 `claude --init-only` / `-p --init` / `-p --maintenance` 时触发，常规交互启动不触发）。原分析称其不存在，不正确。本次保留 Setup hook 注册（用作 init 时的健康检查），并按正确 schema 写入。
2. **Stop 事件输入包含 `assistant_message` 字段**。原分析以为只有 `transcript_path` 需解析 JSONL，实际只需字段名映射，修复成本更低。

---

## 4. 接口设计评估

- **HTTP API 风格不统一**（遗留建议 §6-R3）：`/v1/memory:search`、`/v1/eval/recall` 带版本前缀 + Google 风格 `:action`，其余 110+ 路由是 flat 路径（`/sessions`、`/facts`、`/read`）。architecture.md §3.2 自己也承认。建议统一到 `/v1` 并保留旧路径 alias，越晚改代价越大。
- **错误协议清晰**：`{error: {category, message, retryable}}` 一致且带 retryable 提示，good。
- **MCP 工具面（43 个）偏大**：token 成本已被 CLI+Skill 路线对冲，但 MCP 模式下 schema 注入依旧 ~1k tokens/request。建议长期收敛 MCP 工具到核心 7-10 个（guide/search/resolve/store/commit/timeline/observations），管理面只留 CLI。
- **双 MCP 面（SDK 43 工具 vs mfs-mcp 33 工具）**：职责划分（agent vs ops）合理，但两套工具名容易漂移，建议在 CI 加一个清单对照测试。
- **CLI 设计**：flat 命令 + 三档输出（default/json/verbose）+ 明确退出码约定，质量好；flag→API 字段的非直观映射已在文档显式登记，good practice。

## 5. 代码质量评估

- **Rust**：fmt + clippy `-D warnings` 全量过 CI；错误类型统一（MfsError + AppError）；auth/路径校验/PID lock 等基础设施认真。主要扣分项是读路径的 O(N) 内存重排（§6-R1）与个别 handler 文件偏大（http/mod.rs 1677 行，但其中多为路由注册，可接受）。
- **TypeScript**：hooks 代码风格一致、fail-open 纪律好；但适配层对宿主协议的**事实性认知错误**（hooks schema、Stop 字段、PreToolUse 输出语义）说明缺一个"对照官方文档的 contract 测试层"。`buildSessionMemory`/`computeMetadata` 的启发式提取实现合理（双语关键词、上限裁剪）。
- **测试结构性缺口**：e2e 用 stdin 直调 hook 脚本，验证了 hook 逻辑却没验证 hook **注册**。这是 P0-1 能存活的根本原因。建议增加一条用真实 `claude` CLI（`claude --init-only` 或 `-p` 模式）跑通 SessionStart 注入的冒烟用例（CI 无凭据时 skip）。

## 6. 遗留建议（本次未修，按优先级）

> 已整理为可跟踪的待办清单：[TODO.md](TODO.md)（含验收标准），后续进展请在该文件勾选。

- **R1 读路径扩展性**：episodes 召回改走 semantic.sqlite 的 vec0 索引（或至少给 `get_episodes_by_user` 加 LIMIT + 时间窗），recall 写回改为批量/异步（spawn_blocking + 单事务）。
- **R2 观察捕获开销**：PostToolUse 客户端缓冲合批；`ensureSession` 幂等化后移除每次前置 POST；或服务端 observations 接口支持 upsert-session 语义。
- **R3 API 版本统一**：全部路由迁移 `/v1` 前缀，旧 flat 路径 301/alias 一个大版本周期。
- **R4 真实宿主 e2e**：CI 加 `claude --init-only` 冒烟（见 §5）；Codex 侧探测 `codex` CLI 是否支持 hooks 并在安装输出中区分。
- **R5 migrations 目录说明**：标注每个编号属于 metadata.sqlite 还是 canvas 库（P2-3）。
- **R6 MCP 工具面收敛**：见 §4。

---

## 附：端口统一方案（本次已实施）

- **全局默认端口：18720**。
- **唯一事实来源**：
  - TypeScript：`sdk/src/shared/config.ts` 导出 `DEFAULT_PORT` / `DEFAULT_SERVER_URL`，`install.ts`、`service/manager.ts`、`cli/index.ts` 等一律引用，不再各自 hardcode；
  - Rust：`mfs-types` 导出 `DEFAULT_PORT` / `DEFAULT_BIND_ADDR`，`mfs-server/runtime_config.rs` 与 `mfs-cli/skill.rs` 引用。
- **运行时覆盖**：`.env`（由 `run-server.sh` / `mfs-server --env-file` / docker compose 读取）中的 `MEMFUSE_BIND_ADDR` / `MEMFUSE_SERVER_URL` 始终优先于内置默认值——改端口只需改 `.env` 一处。
- Docker：compose 使用 `${MEMFUSE_PORT:-18720}` 变量替换（compose 自动读取同目录 `.env`），Dockerfile 默认 18720。
