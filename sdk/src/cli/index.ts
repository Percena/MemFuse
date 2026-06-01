/**
 * MemFuse CLI — Token-efficient command-line interface
 *
 * Architecture: Client-Daemon model. CLI is a thin HTTP client that
 * connects to the MemFuse HTTP server (daemon). Each invocation =
 * parse args → HTTP request → format output → exit.
 *
 * See docs/architecture.md §10 CLI Architecture for full design specification.
 */

import { loadConfig, MemFuseConfig } from '../shared/config.js';
import { OutputMode } from './output.js';
import { CliArgs, RegisterFn } from './types.js';
import { registerCoreCommands } from './commands/core.js';
import { registerDigCommands } from './commands/dig.js';
import { registerResourceCommands } from './commands/resources.js';
import { registerHeuristicCommands } from './commands/heuristics.js';
import { registerMemoryCommands } from './commands/memory.js';
import { registerSkillCommands } from './commands/skills.js';
import { registerSessionCommands } from './commands/session.js';
import { registerSystemCommands } from './commands/system.js';
import { registerFactCommands } from './commands/facts.js';
import { registerHeuristicExtendedCommands } from './commands/heuristics-extended.js';
import { registerWorkspaceCommands } from './commands/workspace.js';
import { registerRelationCommands } from './commands/relations.js';
import { registerWatchCommands } from './commands/watches.js';
import { registerResourceExtendedCommands } from './commands/resources-extended.js';
import { registerCodeSymbolCommands } from './commands/code_symbols.js';
import { registerCanvasCommands } from './commands/canvas.js';

// ─── Command Registry ───────────────────────────────────────────────

const COMMANDS: Record<string, (args: CliArgs) => Promise<void>> = {};

const register: RegisterFn = (name: string, handler: (args: CliArgs) => Promise<void>) => {
  COMMANDS[name] = handler;
};

registerCoreCommands(register);
registerDigCommands(register);
registerResourceCommands(register);
registerHeuristicCommands(register);
registerMemoryCommands(register);
registerSkillCommands(register);
registerSessionCommands(register);
registerSystemCommands(register);
registerFactCommands(register);
registerHeuristicExtendedCommands(register);
registerWorkspaceCommands(register);
registerRelationCommands(register);
registerWatchCommands(register);
registerResourceExtendedCommands(register);
registerCodeSymbolCommands(register);
registerCanvasCommands(register);

// ─── Arg Parser ─────────────────────────────────────────────────────

function parseArgs(raw: string[]): CliArgs {
  const command = raw[0] || '';
  const positional: string[] = [];
  const options: Record<string, unknown> = {};
  let mode: OutputMode = 'default';
  let serverOverride: string | undefined;
  let userOverride: string | undefined;
  let sessionOverride: string | undefined;
  let sessionExplicit = false;
  let apiKeyOverride: string | undefined;

  for (let i = 1; i < raw.length; i++) {
    const arg = raw[i];

    // Handle --key=value format (split into --key + value)
    if (arg.startsWith('--') && arg.includes('=')) {
      const [key, value] = arg.split('=');
      const flag = key.slice(2);
      if (flag === 'json') { mode = 'json'; continue; }
      if (flag === 'verbose') { mode = 'verbose'; continue; }
      if (flag === 'server') { serverOverride = value; continue; }
      if (flag === 'user') { userOverride = value; continue; }
      if (flag === 'session') { sessionOverride = value; sessionExplicit = true; continue; }
      if (flag === 'api-key') { apiKeyOverride = value; continue; }
      options[flag] = value;
      continue;
    }

    if (arg === '--json') {
      mode = 'json';
    } else if (arg === '--verbose') {
      mode = 'verbose';
    } else if (arg === '--server' && i + 1 < raw.length) {
      serverOverride = raw[++i];
    } else if (arg === '--user' && i + 1 < raw.length) {
      userOverride = raw[++i];
    } else if (arg === '--session' && i + 1 < raw.length) {
      sessionOverride = raw[++i];
      sessionExplicit = true;
    } else if (arg === '--api-key' && i + 1 < raw.length) {
      apiKeyOverride = raw[++i];
    } else if (arg.startsWith('--') && i + 1 < raw.length && !raw[i + 1].startsWith('--')) {
      options[arg.slice(2)] = raw[++i];
    } else if (arg.startsWith('--')) {
      options[arg.slice(2)] = true;
    } else {
      positional.push(arg);
    }
  }

  // Merge overrides with env config
  const envConfig = loadConfig();
  const threadId = process.env['MEMFUSE_THREAD_ID'];
  const config: MemFuseConfig = {
    serverUrl: serverOverride || envConfig.serverUrl,
    cloudUrl: envConfig.cloudUrl,
    localCanvasUrl: envConfig.localCanvasUrl,
    userId: userOverride || envConfig.userId,
    sessionId: sessionOverride || envConfig.sessionId || threadId || 'default',
    authToken: envConfig.authToken,
  };

  // If api-key override provided, store in args (not process.env)
  return { command, positional, options, mode, config, apiKey: apiKeyOverride, sessionExplicit };
}

// ─── Version & Help ─────────────────────────────────────────────────

register('--version', async () => {
  // Read version from package.json at runtime
  const fs = await import('node:fs/promises');
  const path = await import('node:path');
  const pkgPath = path.join(path.dirname(new URL(import.meta.url).pathname), '..', '..', 'package.json');
  try {
    const pkg = JSON.parse(await fs.readFile(pkgPath, 'utf-8'));
    console.log(`memfuse v${pkg.version || '0.1.0'}`);
  } catch (_) {
    console.log('memfuse v0.1.0');
  }
});

register('--help', async () => {
  console.log(HELP_TEXT);
});

register('help', async () => {
  console.log(HELP_TEXT);
});

// ─── Main Entry ──────────────────────────────────────────────────────

export async function runCli(): Promise<void> {
  const rawArgs = process.argv.slice(2);
  if (rawArgs.length === 0) {
    console.log(HELP_TEXT);
    process.exit(0);
  }

  const args = parseArgs(rawArgs);
  const handler = COMMANDS[args.command];

  if (!handler) {
    console.error(`Unknown command: ${args.command}. Run 'memfuse --help' for available commands.`);
    process.exit(1);
  }

  await handler(args);
}

// ─── Help Text ──────────────────────────────────────────────────────

const HELP_TEXT = `MemFuse CLI — Memory and knowledge management for coding agents

Usage: memfuse <command> [options]

Core [LOOK]:
  resolve-context <query>    Get directional memory signals (--budget N, --strategy, --at-time, --resource-id)
  inject-context <query>     Inject context (alias of resolve-context) (--budget N, --strategy, --at-time, --resource-id)
  search <query>             Search episodic memories (--limit N, --top-k N, --strategy, --thread-id)
  list-facts                 List active facts

DIG (inspect resources):
  ls <uri>                   List directory
  read <uri>                 Read full content
  abstract <uri>             L0 summary
  overview <uri>             L1 overview
  glob <uri> <pattern>       Glob match files
  grep <query>               Keyword grep (--target, --limit)
  find <query>               Path-based search (--target)
  search-context <query>     Context-aware search (--target, --session-context)
  timeline <episode-id>      Chronological context (--direction, --radius)
  get-observations <ids>     Full episode details (comma-separated IDs)
  tree [--uri] [--depth N]   Tree view of resource hierarchy
  stat [--uri]               Resource metadata/statistics
  rebuild                    Rebuild search index
  refresh                    Refresh search index
  trace-fact <id>            Trace fact provenance
  code-symbols-list              List code symbol views (--projection-view-id, --canonical-uri)
  code-symbols-search <query>    Search code symbols (--projection-view-id required)
  code-symbols-create <uri>      Create code symbol view (--projection-view-id required, --symbols, --symbol-types, --signatures, --docstrings)
  code-symbols-delete <view-id>  Delete code symbol view

SAVE (store & commit):
  store-observation <text>   Save observation (--type T, --input, --output, --source-trust, --metadata JSON)
  commit-session             Commit session to trigger consolidation (--reason)
  create-fact <subj> <pred> <val>  Create fact (--id, --confidence, --value-type, --agent-id, --source-assertion-id, --source-episode-ids)
  supersede-fact <id>               Supersede fact with new fact (--new-fact-id)
  retract-fact <id>                 Retract fact
  cite-memories              Mark useful memories (--episode-ids, --fact-ids)
  mkdir <uri>                Create directory
  write <uri> --content      Write content to file
  mv <from> <to>             Move/rename
  rm <uri>                   Delete file/directory
  link <from> <to>              Create relation (--relation-type)
  unlink <from> <to>            Remove relation (--relation-type)
  confirm-rule <rule-id>        Mark rule as confirmed
  create-rule <text>            Create heuristic rule (--tags, --counter-examples, --lifecycle-stage)
  promote-rule <rule-id>        Promote rule (--new-stage draft/candidate/confirmed/archived)
  create-instance <context-summary>  Create instance (--rule-id, --user-reaction, --signal-type, --tags, --agent-proposal, --outcome, --session-id)

Resources [SAVE]:
  add-resource               Add resource (--source-kind, --source-path, --url, --file-name, --content, --revision)
  add-repo <path|url>        Add git repo (--logical-name, --branch, --revision)
  add-inline <name> <text>   Add inline content (--logical-name)
  add-batch                  Batch add resources (--paths comma-separated, --source-kind)
  resources-list             List all resources
  resource-refresh <id>      Refresh resource
  resource-rebuild <id>      Rebuild resource
  resource-export <id>       Export resource (--output-path required)
  resource-import <path>     Import resource pack (--name)
  task-status <task-key>     Check background task
  tasks-list                 List recent tasks (--limit)
  wait-task <task-key>       Wait for task completion (--timeout-ms, --poll-ms)
  evict-tasks                Evict stale completed tasks
  snapshots                  List snapshots (--limit)
  audit                      View audit log (--limit)

Memory Management [SAVE]:
  export-memories            Export as Markdown
  import-memories            Import from Markdown (--file <path> or stdin)
  consolidate                Trigger memory consolidation (--session-id, --resource-id)
  extract-facts              Extract facts from text (--texts or positional args)
  archive                    Archive cold episodes (--hotness-threshold, --min-age-days)
  eval-recall <query>        Evaluate recall accuracy (--expected-facts, --k)

Heuristics [LOOK+SAVE]:
  simulate-reaction <scenario>  Predict user reaction (--tags)
  heuristics-l0                 Top confirmed rules (--max-rules)
  list-rules                    List rules (--lifecycle-stage)
  get-rule <rule-id>            Get rule detail
  list-instances                List instances (--rule-id)
  get-instance <instance-id>    Get instance detail
  retrieve <intent>             Retrieve matching heuristics (--tags, --top-k)

Watches [SAVE]:
  watches-list                  List all watches
  resource-watch <id>           Register watch (--interval)
  resource-watch-disable <id>   Disable watch
  resource-watch-run <id>       Run watch once
  watch-run-due                 Run all due watches
  watch-run-loop                Run watch loop (--iterations, --sleep-ms)
  watch-daemon-start            Start watch daemon (--poll-ms)
  watch-daemon-status           Check watch daemon status
  watch-daemon-stop             Stop watch daemon

Sessions [SAVE lifecycle]:
  session-create              Create session (--session <id>)
  session-list                List sessions
  session-get <id>            Get session detail
  session-context <id>        Get assembled context (--token-budget)
  session-archive <id> <archive-id> Get session archive
  session-delete <id>         Delete session
  add-message <session-id> <content>  Add message to session (--role user/assistant)
  used-context <session-id> <uri>     Record context usage
  used-skill <session-id> <skill-uri> Record skill usage (--success)
  used-tool <session-id> <tool-uri>   Record tool usage (--success)
  session-timeline <session-id>       Get session timeline

Relations [LOOK]:
  relations <uri>               List relations for URI (--limit)

Skills & System [LOOK+SAVE]:
  skills-list                 List skills
  add-skill <path>            Ingest skill
  system-status               System overview
  observer-status             Runtime status
  health                      Check server connectivity
  ready                       Check server readiness
  metrics                     Get server metrics
  service <action>            Local service supervisor control (status/install/start/stop/logs/doctor/uninstall)

Manifest & Canvas [LOOK+SAVE]:
  repo-manifest --repo <repo-id> Get full repo manifest JSON summary
  manifest-get <repo-id>       Get repo manifest identity (legacy alias)
  manifest-update <repo-id>    Update repo manifest (--resource-uri, --default-branch, --primary-languages, --manifest-yaml-path)
  canvas-query --repo <repo-id> Query canvas nodes/edges/overlays (--component, --type structural|contracts|status, --node-type, --status)
  canvas-refresh <repo-id>     Refresh canvas (re-parse repo)
  canvas-snapshot <repo-id>    Create immutable snapshot (--snapshot-type, --merge-commit)
  overlay-propose --repo <repo-id> --tracker github_projects --content-id <id> --identifier <identifier> --type <type> --content-json <json>
  overlay-accept <overlay-id>  Accept overlay (human-only)
  overlay-implement <overlay-id>  Mark overlay implemented (--agent-session-id)
  overlay-abandon <overlay-id>  Abandon overlay (--reason, --abandoner human|agent)
  overlay-conflict --repo <repo-id> --overlay-a <id> --overlay-b <id>  Check conflict between overlays (--description)
  overlay-consolidate <repo-id>   Consolidate implemented overlays (--merge-commit)
  overlays <repo-id>           List overlays (--status)

Setup:
  install                     Deploy skills/hooks (--skills, --hooks, --no-mcp, --platform)

Global options:
  --json                      Raw JSON output (matches HTTP API response format)
  --verbose                   Full markdown output (with emoji, tips)
  --server <url>              Server URL (default: MEMFUSE_SERVER_URL or http://127.0.0.1:8720)
  --user <id>                 User ID (default: MEMFUSE_USER_ID or $USER)
  --session <id>              Session ID (default: MEMFUSE_SESSION_ID or MEMFUSE_THREAD_ID or 'default')
  --api-key <key>             API key (default: MEMFUSE_API_KEY)
  --help                      Show this help
  --version                   Show version

Workflow: LOOK → DIG → SAVE
  1. memfuse inject-context "what am I working on?"  [LOOK]
  2. memfuse abstract mfs://resources/localfs/docs/feature.md  [DIG]
  3. memfuse store-observation "Decided to use pytest" --type Decision  [SAVE]

Search strategies (--strategy):
  precision     Default — relevance-only ranking
  diverse       MMR (λ=0.7) — diversity + relevance balance
  recent        Recency boost (24h 2.0×, 7d 1.3×)
  comprehensive Budget ×2 — exhaustive coverage

Error format:
  HTTP error responses use unified nested format: { "error": { "category", "message", "retryable" } }
  Inline errors in batch/task results use the same nested format.
  Use --json for machine-parseable output.
`;
