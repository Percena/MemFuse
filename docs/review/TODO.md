# MemFuse 待办事项（Review Backlog）

> 来源：[2026-06-10 全面 Review](2026-06-10-comprehensive-review.md) §6 遗留建议。
> P0/P1 及部分 P2 问题已在该次 review 中修复（见 CHANGELOG Unreleased 段），本文只跟踪**尚未实施**的优化项。
> 状态约定：`[ ]` 未开始 / `[~]` 进行中 / `[x]` 完成（完成后请同步勾选并注明 commit）。

## 高优先级

- [x] **R1 读路径扩展性：episodes 召回走向量索引**
  现状：`mfs-memory/src/service.rs::resolve_context` 将用户全部 episodes 拉入内存，
  逐条 `parse_embedding_json` 计算余弦相似度，绕开了 `semantic.sqlite` 现成的 vec0 索引；
  同请求内还对每条命中做同步 SQLite UPDATE（recall 写回）。
  已完成：
  1. `MetadataStore::get_episode_candidates_for_recall` 增加 storage 层候选召回接口，按 account/user/resource、`archived_at IS NULL`、时间窗和 `LIMIT` 预筛，并通过 0023 复合索引支撑 resource/no-resource 两种召回路径，避免 `resolve_context` 拉取用户全量 episodes；
  2. `resolve_context` 默认只取最近 180 天内的 `DEFAULT_EPISODIC_CANDIDATE_K` 个候选，再进行 embedding rerank / budget cap；
  3. recall 写回合并为 `record_recall_access_batch` 单事务；
  4. 显式 recall 的统计写回改为后台 `spawn_blocking` 执行，失败只记录 warning，不阻塞 `/context/resolve` 响应。
  目标：
  1. episodes 召回改走 vec0 向量索引（或至少 `get_episodes_by_user` 加 LIMIT + 时间窗预筛）；
  2. recall 写回改为批量单事务，并移出请求关键路径（spawn_blocking / 后台队列）。
  验收：1000+ episodes 时 `POST /context/resolve` P95 延迟不随记忆量线性增长。

- [x] **R2 观察捕获开销：PostToolUse 合批 + ensureSession 幂等化**
  现状：Claude Code 每个工具调用 spawn 一个 node 进程并发出 2 个 HTTP 请求
  （`ensureSession` 前置 POST + observation POST）。
  已完成：
  1. `POST /sessions/{id}/observations` 在 session 不存在时自动创建同 ID session；
  2. PostToolUse / Stop hooks 删除 observation 写入前的 `ensureSession` 前置请求；
  3. SDK 回归测试断言 PostToolUse 单次工具调用只发 1 个 observation HTTP 请求。
  说明：低价值观察本地 spool 合批作为后续吞吐优化保留；当前验收目标“单次工具调用 hook 开销 ≤ 1 个 HTTP 请求”已由服务端 upsert + 客户端去前置请求达成。
  目标：
  1. 服务端 `POST /sessions/{id}/observations` 支持 session 不存在时自动创建（upsert 语义），
     客户端删除 `ensureSession` 前置请求；
  2. 低价值观察客户端本地缓冲、按条数/时间窗合批提交（hook 进程短命，可落地为本地 spool 文件 + 下次 hook 顺带冲刷）。
  验收：单次工具调用的 hook 开销 ≤ 1 个 HTTP 请求。

- [~] **R4 真实宿主 e2e：hook 注册路径冒烟测试**
  现状：e2e 测试直接向 hook 脚本 stdin 喂 JSON，绕过了 Claude Code 的真实 hook 注册——
  这是 P0-1（hooks 格式错误）能长期存活的根本原因。
  已推进：
  1. `sdk/tests/sdk.test.mjs` 已增加 Claude Code installer 回归测试，断言写出的 hooks 必须是按事件分组的 canonical schema，且不允许回退到旧 `hooks: [{event, cmd}]` 数组；
  2. Codex installer 已改为执行 `codex features list` 探测 `hooks` / `codex_hooks` 支持，并在安装输出中区分 supported / unsupported / unknown；
  3. `sdk/tests/sdk.test.mjs` 已用 fake Codex CLI 覆盖 hooks-supported 分支。
  未完成：
  1. 仍需 CI 增加真实 `claude` CLI 冒烟（无凭据时 skip），验证宿主实际读取 `.claude/settings.local.json` 并触发 SessionStart 注入。
  目标：
  1. CI 增加一条用真实 `claude` CLI（`claude --init-only` 或 `-p` 模式）跑通
     memfuse-setup → SessionStart 注入的冒烟用例（无凭据时 skip）；
  2. Codex 侧在安装时探测 CLI 是否支持 hooks（目前只打印提示），并在 e2e 中区分两种结果。
  验收：安装器写出的配置若与宿主 schema 不符，CI 必须失败。

## 中优先级

- [ ] **R3 API 版本统一：全部路由迁移 `/v1` 前缀**
  现状：仅 `/v1/memory:search`、`/v1/eval/recall` 等少数路由带版本前缀，
  其余 110+ 路由是 flat 路径（`/sessions`、`/facts`、`/read`），architecture.md §3.2 已自述不一致。
  目标：所有路由提供 `/v1/...` 规范路径，旧 flat 路径保留 alias 一个大版本周期后移除；
  SDK（hooks/MCP/CLI 的 `PATHS` 常量）同步切换。
  验收：`docs/architecture.md` §3.2 路由表全部以 `/v1` 开头，旧路径标记 deprecated。

- [ ] **R6 MCP 工具面收敛**
  现状：SDK MCP 暴露 43 个工具，schema 注入每请求约 ~1k tokens；CLI+Skill 已是主推路径。
  目标：MCP 默认只注册核心工具（guide / search_memories / resolve_context /
  store_observation / commit_session / timeline / get_observations），
  其余移到 `MEMFUSE_MCP_FULL=1` 之类的 opt-in 开关后面；管理面统一走 CLI。
  验收：默认 MCP 模式 schema 注入 token 量下降 ≥ 60%，e2e 不回归。

## 低优先级

- [ ] **R5 migrations 编号说明**
  现状：`crates/mfs-metadata/src/migrations/` 编号存在按库分流的重复变体
  （0013×2、0014×3、0017×2）且缺 0004/0021，新贡献者难以判断归属。
  目标：迁移目录内补一份 README，说明每个编号属于 metadata.sqlite 还是 canvas 库、
  缺号原因；或按子目录拆分两套迁移序列。

- [ ] **双 MCP 面工具清单对照测试**
  现状：SDK MCP（43 agent 工具）与 mfs-mcp crate（33 ops 工具）约定互不重名，但无 CI 保证。
  目标：CI 增加一个清单对照测试，两边工具名出现交集即失败。

- [ ] **Stop hook 的 transcript 兜底**
  现状：Stop hook 已改读 `assistant_message`；当宿主未提供该字段时（旧版本 Claude Code
  或其他平台），可再从 `transcript_path` JSONL 解析最后一条 assistant 消息作为兜底。

- [ ] **UserPromptSubmit 轻量化端点**
  现状：每个 prompt 都触发完整 resolve_context（含 embedding + intent 分类），
  hook timeout 5s 内偶发超时即放弃注入。
  目标：服务端提供一个低延迟的 `/context/signal` 轻量端点（仅 facts 置信度过滤 + FTS，
  无 embedding/LLM），UserPromptSubmit 切换到该端点。
