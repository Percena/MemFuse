# MemFuse CLI Command Reference

Load this reference only when the main skill does not contain the command or flag you need.

## Core

| Command | Purpose |
| --- | --- |
| `memfuse resolve-context <query>` | Pull directional memory signals. Flags: `--budget N`, `--strategy precision|diverse|recent|comprehensive`, `--at-time ISO8601`, `--resource-id`. |
| `memfuse inject-context <query>` | Alias of `resolve-context`; prefer `resolve-context`. |
| `memfuse search <query>` | Search episodic memories. Flags: `--limit N`, `--top-k N`, `--strategy`, `--thread-id`. |
| `memfuse store-observation [text]` | Save an observation. Flags: `--type`, `--input`, `--output`, `--source-trust`, `--metadata JSON`. |
| `memfuse commit-session` | Trigger consolidation. Flags: `--reason`. |

## Inspect Resources

| Command | Purpose |
| --- | --- |
| `memfuse ls <uri>` | List directory entries. |
| `memfuse read <uri>` | Read full content. |
| `memfuse abstract <uri>` | Read L0 summary. |
| `memfuse overview <uri>` | Read L1 overview. |
| `memfuse glob <uri> <pattern>` | Glob match files. |
| `memfuse grep <query>` | Keyword grep. Flags: `--target`, `--limit`. |
| `memfuse find <query>` | Path-based search. Flags: `--target`. |
| `memfuse search-context <query>` | Context-aware search. Flags: `--target`, `--session-context`. |
| `memfuse timeline <episode-id>` | Chronological context. Flags: `--direction`, `--radius`. |
| `memfuse get-observations <ids>` | Full episode details for comma-separated IDs. |
| `memfuse tree [--uri] [--depth N]` | Tree view of resource hierarchy. |
| `memfuse stat [--uri]` | Resource metadata and statistics. |

## Facts

| Command | Purpose |
| --- | --- |
| `memfuse list-facts` | List active facts. |
| `memfuse create-fact <subj> <pred> <val>` | Create a fact. Flags: `--id`, `--confidence`, `--value-type`, `--agent-id`, `--source-assertion-id`, `--source-episode-ids`. |
| `memfuse supersede-fact <id>` | Supersede a fact. Flags: `--new-fact-id`, `--subject`, `--predicate`, `--value`. |
| `memfuse retract-fact <id>` | Retract a fact. Flags: `--reason`. |
| `memfuse trace-fact <id>` | Trace fact provenance to source episodes. |
| `memfuse cite-memories` | Mark useful memories. Flags: `--episode-ids`, `--fact-ids`. |

## Memory Import And Export

| Command | Purpose |
| --- | --- |
| `memfuse export-memories` | Export memories as Markdown. |
| `memfuse import-memories --file <path>` | Import memories from Markdown. |
| Pipe stdin to `memfuse import-memories` | Import memories from stdin. |
| `memfuse consolidate` | Trigger memory consolidation. Flags: `--session-id`, `--resource-id`. |
| `memfuse extract-facts` | Extract facts from text. Flags: `--texts` or positional args. |
| `memfuse archive` | Archive cold episodes. Flags: `--hotness-threshold`, `--min-age-days`. |
| `memfuse eval-recall <query>` | Evaluate recall accuracy. Flags: `--expected-facts`, `--k`. |

## Resource Management

| Command | Purpose |
| --- | --- |
| `memfuse add-resource` | Add resource. Flags: `--source-kind`, `--source-path`, `--url`, `--file-name`, `--content`, `--revision`. |
| `memfuse add-repo <path|url>` | Add git repo. Flags: `--logical-name`, `--branch`, `--revision`. |
| `memfuse add-inline <name> <text>` | Add inline content. Flags: `--logical-name`. |
| `memfuse add-batch` | Batch add resources. Flags: `--paths`, `--source-kind`. |
| `memfuse resources-list` | List resources. |
| `memfuse resource-refresh <id>` | Refresh resource. |
| `memfuse resource-rebuild <id>` | Rebuild resource. |
| `memfuse resource-export <id>` | Export resource. Flags: `--output-path`. |
| `memfuse resource-import <path>` | Import resource pack. Flags: `--name`. |
| `memfuse task-status <task-key>` | Check background task. |
| `memfuse tasks-list` | List recent tasks. Flags: `--limit`. |
| `memfuse wait-task <task-key>` | Wait for task completion. Flags: `--timeout-ms`, `--poll-ms`. |
| `memfuse evict-tasks` | Evict stale completed tasks. |
| `memfuse snapshots` | List snapshots. Flags: `--limit`. |
| `memfuse audit` | View audit log. Flags: `--limit`. |

## Heuristics And Skills

| Command | Purpose |
| --- | --- |
| `memfuse simulate-reaction <scenario>` | Predict user reaction. Flags: `--tags`. |
| `memfuse heuristics-l0` | Top confirmed rules. Flags: `--max-rules`. |
| `memfuse confirm-rule <rule-id>` | Mark rule as confirmed. |
| `memfuse create-rule <text>` | Create heuristic rule. Flags: `--tags`, `--counter-examples`, `--lifecycle-stage`, `--evidence-threshold`. |
| `memfuse list-rules` | List rules. Flags: `--lifecycle-stage`. |
| `memfuse get-rule <rule-id>` | Get rule detail. |
| `memfuse promote-rule <rule-id>` | Promote rule. Flags: `--new-stage draft|candidate|confirmed|archived`. |
| `memfuse create-instance <context-summary>` | Create heuristic instance. Flags: `--rule-id`, `--user-reaction`, `--signal-type`, `--tags`, `--agent-proposal`, `--outcome`, `--session-id`. |
| `memfuse list-instances` | List instances. Flags: `--rule-id`. |
| `memfuse get-instance <instance-id>` | Get instance detail. |
| `memfuse retrieve <intent>` | Retrieve matching heuristics. Flags: `--tags`, `--top-k`. |
| `memfuse skills-list` | List registered skills. |
| `memfuse add-skill <path>` | Ingest a skill. |

## Workspace Writes

| Command | Purpose |
| --- | --- |
| `memfuse mkdir <uri>` | Create directory. |
| `memfuse write <uri> --content <text>` | Write content to file. |
| `memfuse mv <from> <to>` | Move or rename. |
| `memfuse rm <uri>` | Delete file or directory. |
| `memfuse rebuild` | Trigger a workspace-wide rebuild. |
| `memfuse refresh` | Trigger a workspace-wide refresh. |

## Relations

| Command | Purpose |
| --- | --- |
| `memfuse link <from> <to>` | Create relation. Flags: `--relation-type`. |
| `memfuse unlink <from> <to>` | Remove relation. Flags: `--relation-type`. |
| `memfuse relations <uri>` | List relations. Flags: `--limit`. |

## Watches

| Command | Purpose |
| --- | --- |
| `memfuse watches-list` | List watches. |
| `memfuse resource-watch <id>` | Register watch. Flags: `--interval`. |
| `memfuse resource-watch-disable <id>` | Disable watch. |
| `memfuse resource-watch-run <id>` | Run watch once. |
| `memfuse watch-run-due` | Run all due watches. |
| `memfuse watch-run-loop` | Run watch loop. Flags: `--iterations`, `--sleep-ms`. |
| `memfuse watch-daemon-start` | Start watch daemon. Flags: `--poll-ms`. |
| `memfuse watch-daemon-status` | Check daemon status. |
| `memfuse watch-daemon-stop` | Stop daemon. |

## Sessions

| Command | Purpose |
| --- | --- |
| `memfuse session-create` | Create session. Flags: `--session <id>`. |
| `memfuse session-list` | List sessions. |
| `memfuse session-get <id>` | Get session detail. |
| `memfuse session-context <id>` | Get assembled context. Flags: `--token-budget`. |
| `memfuse session-archive <id> <archive-id>` | Get session archive. |
| `memfuse session-delete <id>` | Delete session. |
| `memfuse add-message <session-id> <content>` | Add message. Flags: `--role user|assistant`. |
| `memfuse used-context <session-id> <uri>` | Record context usage. |
| `memfuse used-skill <session-id> <skill-uri>` | Record skill usage. Flags: `--success`. |
| `memfuse used-tool <session-id> <tool-uri>` | Record tool usage. Flags: `--success`. |
| `memfuse session-timeline <session-id>` | Get session timeline. |

## Code Symbols

| Command | Purpose |
| --- | --- |
| `memfuse code-symbols-list` | List code symbol views. Flags: `--projection-view-id`, `--canonical-uri`. |
| `memfuse code-symbols-search <query>` | Search code symbols. Requires `--projection-view-id`. |
| `memfuse code-symbols-create <uri>` | Create code symbol view. Flags: `--projection-view-id`, `--symbols`, `--symbol-types`, `--signatures`, `--docstrings`. |
| `memfuse code-symbols-delete <view-id>` | Delete code symbol view. |

## System

| Command | Purpose |
| --- | --- |
| `memfuse health` | Check server connectivity. |
| `memfuse ready` | Check server readiness. |
| `memfuse metrics` | Get server metrics. |
| `memfuse system-status` | System overview. |
| `memfuse observer-status` | Runtime status. |

## Setup

| Command | Purpose |
| --- | --- |
| `memfuse install` | Deploy skills, hooks, and MCP. Flags: `--skills`, `--hooks`, `--no-mcp`, `--platform`. |

## Global Options

| Option | Purpose |
| --- | --- |
| `--json` | Raw JSON output. |
| `--verbose` | Full markdown output. |
| `--server <url>` | Override server URL. |
| `--user <id>` | Override user ID. |
| `--session <id>` | Override session ID. |
| `--api-key <key>` | Override API key. |
