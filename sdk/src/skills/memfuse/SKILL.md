---
name: memfuse
description: Use when coding agents need to retrieve prior project context, inspect indexed resources, save durable decisions or findings, and preserve multi-turn or cross-session continuity through the MemFuse CLI.
allowed-tools: Bash(memfuse:*)
---

# MemFuse CLI

Use MemFuse as active memory for coding work. The CLI talks to the configured MemFuse server and is the preferred memory interface for this skill.

Constraint: use only `Bash(memfuse:*)` for memory operations. Do not use MCP memory tools from this skill.

## Default Workflow

Follow `LOOK -> DIG -> SAVE` for significant tasks.

1. LOOK: pull directional memory signals before work.

```bash
memfuse resolve-context "current task description" --budget 1500
```

2. DIG: inspect the specific resources or episodes that matter.

```bash
memfuse search "relevant decision or bug"
memfuse abstract mfs://resources/project/docs/design.md
memfuse read mfs://resources/project/src/module.rs
memfuse get-observations ep_abc123,ep_def456
```

3. SAVE: store only durable information worth seeing in future sessions.

```bash
memfuse store-observation "Chose SQLite WAL for replay cache durability" --type Decision
memfuse commit-session --reason "finished replay-cache design"
```

## Multi-Turn Protocol

At the start of a new task or resumed conversation, run `resolve-context` with the user's actual task.

When a key intermediate fact appears, save it immediately. Do not wait until the end of a long task.

At natural checkpoints, run `commit-session` so future sessions can retrieve the consolidated state.

If continuing an existing thread, rely on the process environment when available:

- `MEMFUSE_SESSION_ID` has first priority.
- `MEMFUSE_THREAD_ID` can route Codex threads to MemFuse sessions.
- If neither is set and session routing matters, pass `--session <id>` explicitly.

Do not assume a hook can mutate the parent Codex process environment. Hook-created context is passive; CLI session routing still comes from the CLI flags or inherited environment.

## What To Save

Save:

- User preferences that should affect future work.
- Architecture decisions and rejected alternatives.
- Bug root causes and verified fixes.
- Gotchas, migration notes, and operational procedures.
- Important intermediate state needed by a later turn or session.
- Resource IDs, episode IDs, fact IDs, or commands that are useful follow-up anchors.

Do not save:

- Raw logs or noisy command output unless the exact output is diagnostically important.
- Secrets, credentials, tokens, private data, or large pasted content.
- Facts already present in the current MemFuse context.
- Low-value narration like "looked at file X".
- Unverified guesses stated as fact. Save as `Discovery` or `ManualNote` only when clearly provisional.

## Common Commands

```bash
memfuse health
memfuse resolve-context "task" --budget 1500
memfuse search "query" --limit 10
memfuse store-observation "important durable note" --type Decision
memfuse commit-session --reason "checkpoint"
memfuse get-observations ep_abc123,ep_def456
memfuse trace-fact fact_abc123
memfuse simulate-reaction "planned risky change"
```

For the full command reference, read `references/commands.md` only when you need a less common command, exact flags, resource management, facts, watches, imports/exports, code symbols, or workspace write operations.

## Retrieval Guidance

Prefer compact outputs first:

```bash
memfuse resolve-context "auth refactor"
memfuse search "connection pool exhaustion"
```

Drill down only when the signal is relevant:

```bash
memfuse timeline ep_abc123 --radius 5
memfuse get-observations ep_abc123
```

Use strategies deliberately:

- `precision`: default relevance-first lookup.
- `recent`: recent decisions or "what happened lately".
- `diverse`: broad related topics without duplicates.
- `comprehensive`: exhaustive recall when token cost is acceptable.

```bash
memfuse resolve-context "recent testing decisions" --strategy recent
memfuse search "all auth approaches" --strategy diverse
```

For historical state, use point-in-time queries:

```bash
memfuse resolve-context "project preferences" --at-time 2026-04-15T12:00:00Z
```

## Failure Behavior

If `memfuse health` or another memory command fails because the server is offline, continue the user's task using local context. Mention in the final response that MemFuse memory was unavailable and include the command/error briefly.

Do not repeatedly retry unavailable memory commands. One health check plus one task-relevant command is enough evidence.

## Output Modes

- Default: compact, token-efficient text.
- `--json`: machine-readable output for scripts.
- `--verbose`: full markdown detail when the compact output is insufficient.

Use default mode for normal agent work. Use `--json` only when parsing. Use `--verbose` only when the user or task needs detail.

## Observation Types

Use `--type` to make saved memories easier to retrieve:

| Type | Use for |
| --- | --- |
| `Decision` | deliberate choice or rejected alternative |
| `Discovery` | important finding |
| `BugFix` | root cause plus verified fix |
| `Change` | behavior or configuration changed |
| `Feature` | new capability |
| `Refactor` | structural code change |
| `Gotcha` | pitfall or surprising behavior |
| `Pattern` | reusable approach |
| `ManualNote` | other durable note |

Structured example:

```bash
memfuse store-observation --type BugFix --input "pool exhaustion under concurrent search" --output "raised max_conns and added idle timeout"
```

## Hooks Vs Skill

Hooks passively inject context and capture tool observations when configured. This skill is active: you decide when to look up context, inspect details, save important state, and commit checkpoints.

Use hooks as background safety net. Use this skill for intentional memory operations.

## Notes

- `inject-context` is an alias for `resolve-context`; prefer `resolve-context`.
- Array parameters such as episode IDs, fact IDs, and tags are comma-separated.
- `import-memories` should use `--file <path>` or stdin, not a large positional argument.
