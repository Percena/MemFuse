/**
 * MemFuse Setup (Install)
 *
 * Configures MCP server, hooks, skills, and plugin manifests
 * for Claude Code and/or Codex.
 *
 * Install flow (3 + 1 layers):
 *   1. MCP server   — CLI preferred, JSON fallback
 *   2. Hooks        — JSON config (platform-specific format)
 *   3. Skills        — copy to standard skill directories
 *   4. Plugin manifest — copy .claude-plugin/ or .codex-plugin/
 *
 * Platform differences (intentional, minimal):
 *   - Claude Code: 7+1 hooks (incl. PreToolUse[Read] + UserPromptSubmit), .claude/ dirs
 *   - Codex: 3 hooks (no PreToolUse/PreCompact/SessionEnd), .codex/ dirs
 */

import { createHash } from 'node:crypto';
import { readFileSync, writeFileSync, existsSync, mkdirSync, cpSync, realpathSync } from 'node:fs';
import { homedir } from 'node:os';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { execFileSync } from 'node:child_process';
import { installSkill, SKILL_NAMES } from '../skills/loader.js';
import { httpRequest } from '../shared/http.js';
import { DEFAULT_SERVER_URL } from '../shared/config.js';

// ─── Configuration helpers ──────────────────────────────────────────────

interface SetupOptions {
  platform?: 'claude-code' | 'codex' | 'both';
  projectDir?: string;
  userId?: string;
  serverUrl?: string;
}

type CodexHookEventName = 'SessionStart' | 'PostToolUse' | 'Stop';

interface CodexHookSpec {
  eventName: CodexHookEventName;
  matcher?: string;
  command: string;
  timeout: number;
}

const CODEX_HOOK_EVENT_KEYS: Record<CodexHookEventName, string> = {
  SessionStart: 'session_start',
  PostToolUse: 'post_tool_use',
  Stop: 'stop',
};

const CODEX_HOOK_EVENTS_WITH_MATCHERS = new Set<CodexHookEventName>([
  'SessionStart',
  'PostToolUse',
]);

function getSdkRoot(): string {
  return resolve(join(fileURLToPath(import.meta.url), '..', '..', '..'));
}

function ensureJsonFile(file: string): void {
  if (!existsSync(file)) {
    writeFileSync(file, '{}\n', 'utf-8');
  }
}

function getCodexHome(): string {
  return resolve(process.env.CODEX_HOME || join(homedir(), '.codex'));
}

function canonicalJson(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value as Record<string, unknown>)
        .sort()
        .map((key) => [key, canonicalJson((value as Record<string, unknown>)[key])]),
    );
  }
  return value;
}

function codexHookTrustHash(spec: CodexHookSpec): string {
  const group: Record<string, unknown> = {
    event_name: CODEX_HOOK_EVENT_KEYS[spec.eventName],
    hooks: [{
      type: 'command',
      command: spec.command,
      timeout: Math.max(1, spec.timeout),
      async: false,
    }],
  };
  if (CODEX_HOOK_EVENTS_WITH_MATCHERS.has(spec.eventName) && spec.matcher !== undefined) {
    group.matcher = spec.matcher;
  }
  const serialized = JSON.stringify(canonicalJson(group));
  return `sha256:${createHash('sha256').update(serialized).digest('hex')}`;
}

function tomlQuote(value: string): string {
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

function upsertCodexHooksFeature(configToml: string): string {
  const lines = configToml.replace(/\r\n/g, '\n').split('\n');
  const featureHeader = /^\s*\[features]\s*(?:#.*)?$/;
  const anyHeader = /^\s*\[.*]\s*(?:#.*)?$/;
  const start = lines.findIndex((line) => featureHeader.test(line));

  if (start === -1) {
    const prefix = configToml.trimEnd();
    return `${prefix}${prefix ? '\n\n' : ''}[features]\nhooks = true\n`;
  }

  let end = lines.length;
  for (let i = start + 1; i < lines.length; i++) {
    if (anyHeader.test(lines[i] || '')) {
      end = i;
      break;
    }
  }

  let foundHooks = false;
  const nextLines = [...lines.slice(0, start + 1)];
  for (const line of lines.slice(start + 1, end)) {
    if (/^\s*codex_hooks\s*=/.test(line)) continue;
    if (/^\s*hooks\s*=/.test(line)) {
      nextLines.push('hooks = true');
      foundHooks = true;
      continue;
    }
    nextLines.push(line);
  }
  if (!foundHooks) nextLines.splice(start + 1, 0, 'hooks = true');
  nextLines.push(...lines.slice(end));
  return nextLines.join('\n');
}

function removeTomlTables(configToml: string, tableHeaders: Set<string>): string {
  const lines = configToml.replace(/\r\n/g, '\n').split('\n');
  const anyHeader = /^\s*\[.*]\s*(?:#.*)?$/;
  const kept: string[] = [];
  for (let i = 0; i < lines.length;) {
    const trimmed = (lines[i] || '').trim();
    if (tableHeaders.has(trimmed)) {
      i++;
      while (i < lines.length && !anyHeader.test(lines[i] || '')) i++;
      continue;
    }
    kept.push(lines[i] || '');
    i++;
  }
  return kept.join('\n');
}

function writeCodexHookTrustConfig(configFile: string, hooksFile: string, specs: CodexHookSpec[]): void {
  mkdirSync(dirname(configFile), { recursive: true });
  const canonicalHooksFile = realpathSync(hooksFile);
  const hookStates = specs.map((spec) => {
    const eventKey = CODEX_HOOK_EVENT_KEYS[spec.eventName];
    const key = `${canonicalHooksFile}:${eventKey}:0:0`;
    return { key, hash: codexHookTrustHash(spec) };
  });
  const tableHeaders = new Set(hookStates.map(({ key }) => `[hooks.state.${tomlQuote(key)}]`));
  let configToml = existsSync(configFile) ? readFileSync(configFile, 'utf-8') : '';
  configToml = upsertCodexHooksFeature(configToml);
  configToml = removeTomlTables(configToml, tableHeaders).trimEnd();
  const trustBlocks = hookStates
    .map(({ key, hash }) => `[hooks.state.${tomlQuote(key)}]\ntrusted_hash = ${tomlQuote(hash)}`)
    .join('\n\n');
  writeFileSync(configFile, `${configToml}${configToml ? '\n\n' : ''}${trustBlocks}\n`, 'utf-8');
}

function codexCliHooksSupport(): 'supported' | 'unsupported' | 'unknown' {
  try {
    const output = execFileSync('codex', ['features', 'list'], {
      encoding: 'utf-8',
      stdio: ['ignore', 'pipe', 'pipe'],
      timeout: 5000,
    });
    // `codex features list` has no stable column format across builds, so match
    // tolerantly: a row whose name token (ignoring a trailing `:`) is the hooks
    // feature, and treat any affirmative token on that row as "enabled".
    const hooksTokens = output
      .split(/\r?\n/)
      .map((line) => line.trim().split(/\s+/))
      .find((tokens) => {
        const name = tokens[0]?.replace(/:$/, '').toLowerCase();
        return name === 'hooks' || name === 'codex_hooks';
      });
    if (!hooksTokens) return 'unsupported';
    const enabled = hooksTokens.some((token) => /^(true|enabled|yes|on)$/i.test(token));
    return enabled ? 'supported' : 'unsupported';
  } catch {
    return 'unknown';
  }
}

function mergeJson(file: string, snippet: Record<string, unknown>): void {
  let existing: Record<string, unknown> = {};
  if (existsSync(file)) {
    try { existing = JSON.parse(readFileSync(file, 'utf-8')); } catch { existing = {}; }
  }
  const merged = { ...existing, ...snippet };
  for (const key of Object.keys(snippet)) {
    if (typeof snippet[key] === 'object' && !Array.isArray(snippet[key]) && typeof existing[key] === 'object' && !Array.isArray(existing[key])) {
      merged[key] = { ...(existing[key] as Record<string, unknown>), ...(snippet[key] as Record<string, unknown>) };
    }
  }
  writeFileSync(file, JSON.stringify(merged, null, 2) + '\n', 'utf-8');
}

// ─── Hook config merging (Claude Code & Codex shared object format) ─────
//
// Both platforms use the same shape:
//   hooks: { EventName: [{ matcher?, hooks: [{ type: "command", command, timeout }] }] }
// (timeout is in seconds).

interface HookGroup {
  matcher?: string;
  hooks: Array<Record<string, unknown>>;
}

function isMemfuseHookCommand(command: unknown): boolean {
  return /memfuse/i.test(String(command ?? ''));
}

/**
 * Merge platform hook groups into a JSON config file without clobbering
 * user-defined hooks: existing memfuse entries are replaced, everything
 * else is preserved. Also cleans up the malformed `hooks: [...]` array
 * format written by older installers (never valid for either platform).
 */
function mergeHooksConfig(file: string, newHooks: Record<string, HookGroup[]>): void {
  let existing: Record<string, unknown> = {};
  if (existsSync(file)) {
    try { existing = JSON.parse(readFileSync(file, 'utf-8')); } catch { existing = {}; }
  }
  if (Array.isArray(existing.hooks)) {
    // Legacy malformed format from older memfuse installers — drop it.
    delete existing.hooks;
  }
  const hooks: Record<string, unknown> =
    existing.hooks && typeof existing.hooks === 'object'
      ? existing.hooks as Record<string, unknown>
      : {};

  for (const [event, groups] of Object.entries(newHooks)) {
    const current = Array.isArray(hooks[event]) ? hooks[event] as HookGroup[] : [];
    const kept = current
      .map((group) => ({
        ...group,
        hooks: Array.isArray(group.hooks)
          ? group.hooks.filter((h) => !isMemfuseHookCommand(h.command))
          : [],
      }))
      .filter((group) => group.hooks.length > 0);
    hooks[event] = [...kept, ...groups];
  }

  existing.hooks = hooks;
  writeFileSync(file, JSON.stringify(existing, null, 2) + '\n', 'utf-8');
}

/**
 * Build the shell command for a hook script. The platform is passed
 * explicitly via --platform so hooks never need to guess the host from
 * payload heuristics; server URL and user id are inlined as env vars so
 * the values chosen at install time reach the hook process.
 */
function buildHookCommand(
  hooksDir: string,
  script: string,
  platform: 'claude-code' | 'codex',
  envVars: Record<string, string>,
): string {
  const envPrefix = Object.entries(envVars)
    .map(([k, v]) => `${k}="${v.replace(/"/g, '\\"')}"`)
    .join(' ');
  return `${envPrefix} node "${join(hooksDir, script)}" --platform=${platform}`;
}

/** Try to add MCP server via CLI command, return true if succeeded */
function tryMcpCliAdd(cliCmd: string[], serverName: string, command: string, args: string[], envVars: Record<string, string>): boolean {
  try {
    const [bin, ...cliArgs] = cliCmd;
    if (!bin) return false;
    const envArgs = Object.entries(envVars).flatMap(([k, v]) => ['-e', `${k}=${v}`]);
    execFileSync(bin, [...cliArgs, serverName, command, ...envArgs, ...args], { stdio: 'pipe', timeout: 10000 });
    return true;
  } catch { return false; }
}

/** Copy plugin manifest from source to the platform-specific plugin directory */
function installPluginManifest(
  sourcePluginDir: string,
  targetDir: string,
  pluginDirName: '.claude-plugin' | '.codex-plugin',
): string {
  const sourceManifest = join(sourcePluginDir, 'plugin.json');
  if (!existsSync(sourceManifest)) return '';

  const targetPluginDir = join(targetDir, pluginDirName);
  mkdirSync(targetPluginDir, { recursive: true });
  cpSync(sourceManifest, join(targetPluginDir, 'plugin.json'), { force: true });
  return targetPluginDir;
}

// ─── Main setup ─────────────────────────────────────────────────────────

export async function runSetup(args: string[]): Promise<void> {
  const options: SetupOptions = { platform: 'both', projectDir: process.cwd() };

  for (const arg of args) {
    if (arg.startsWith('--platform=')) options.platform = arg.split('=')[1] as SetupOptions['platform'];
    else if (arg.startsWith('--project-dir=')) options.projectDir = resolve(arg.split('=')[1]);
    else if (arg.startsWith('--user-id=')) options.userId = arg.split('=')[1];
    else if (arg.startsWith('--server-url=')) options.serverUrl = arg.split('=')[1];
    else { console.error(`Unknown argument: ${arg}`); process.exit(1); }
  }

  const serverUrl = options.serverUrl || process.env.MEMFUSE_SERVER_URL || DEFAULT_SERVER_URL;
  const userId = options.userId || process.env.MEMFUSE_USER_ID || process.env.USER || 'default';

  console.log('MemFuse Install');
  console.log(`  Platform:    ${options.platform}`);
  console.log(`  Project dir: ${options.projectDir}`);
  console.log(`  User ID:     ${userId}`);
  console.log(`  Server URL:  ${serverUrl}`);
  console.log('');

  const skipMcp = process.env['MEMFUSE_SKIP_MCP'] === '1';
  const installMode = process.env['MEMFUSE_INSTALL_MODE'] || 'full';

  if (options.platform === 'claude-code' || options.platform === 'both') {
    await installClaudeCode(options.projectDir!, serverUrl, userId, skipMcp, installMode);
  }
  if (options.platform === 'codex' || options.platform === 'both') {
    await installCodex(options.projectDir!, serverUrl, userId, skipMcp, installMode);
  }

  await healthCheck(serverUrl);
}

// ─── Claude Code installer ──────────────────────────────────────────────

async function installClaudeCode(projectDir: string, serverUrl: string, userId: string, skipMcp: boolean, installMode: string): Promise<void> {
  console.log('=== Installing for Claude Code ===');

  const sdkRoot = getSdkRoot();
  const mcpServerPath = join(sdkRoot, 'bin', 'memfuse-mcp.cjs');
  const hooksDir = join(sdkRoot, 'bin', 'hooks');
  const claudeDir = join(projectDir, '.claude');
  const skillsDir = join(projectDir, '.claude', 'skills');
  const envVars = { MEMFUSE_USER_ID: userId, MEMFUSE_SERVER_URL: serverUrl };

  const doMcp = !skipMcp;
  const doHooks = installMode === 'full' || installMode === 'skills-hooks' || installMode === 'hooks-only';
  const doSkills = installMode === 'full' || installMode === 'skills-hooks' || installMode === 'skills-only';

  // 1. MCP server — try CLI first, fallback to JSON
  if (doMcp) {
    const cliAdded = tryMcpCliAdd(['claude', 'mcp', 'add'], 'memfuse', 'node', [mcpServerPath], envVars);
    if (cliAdded) {
      console.log('  ✓ MCP server added via `claude mcp add`');
    } else {
      mkdirSync(claudeDir, { recursive: true });
      const settingsFile = join(claudeDir, 'settings.local.json');
      ensureJsonFile(settingsFile);
      mergeJson(settingsFile, { mcpServers: { memfuse: { command: 'node', args: [mcpServerPath], env: envVars } } });
      console.log(`  ✓ MCP server configured in ${settingsFile} (CLI unavailable)`);
    }
  } else {
    console.log('  ⊘ MCP registration skipped (--no-mcp)');
  }

  // 2. Hooks — Claude Code settings schema:
  //    hooks: { EventName: [{ matcher?, hooks: [{ type: "command", command, timeout }] }] }
  if (doHooks) {
    mkdirSync(claudeDir, { recursive: true });
    const settingsFile = join(claudeDir, 'settings.local.json');
    ensureJsonFile(settingsFile);
    const hook = (script: string, timeout: number): Record<string, unknown> => ({
      type: 'command',
      command: buildHookCommand(hooksDir, script, 'claude-code', envVars),
      timeout,
    });
    mergeHooksConfig(settingsFile, {
      SessionStart: [{ hooks: [hook('session-start.cjs', 15)] }],
      UserPromptSubmit: [{ hooks: [hook('user-prompt-submit.cjs', 5)] }],
      PreToolUse: [{ matcher: 'Read', hooks: [hook('pre-tool-use.cjs', 5)] }],
      PostToolUse: [{ hooks: [hook('post-tool-use.cjs', 15)] }],
      Stop: [{ hooks: [hook('stop.cjs', 10)] }],
      PreCompact: [{ hooks: [hook('pre-compact.cjs', 10)] }],
      SessionEnd: [{ hooks: [hook('session-end.cjs', 10)] }],
      // Setup only fires on `claude --init-only` / `-p --init|--maintenance`;
      // used as an installation-time health probe.
      Setup: [{ hooks: [hook('setup.cjs', 10)] }],
    });
    console.log('  ✓ Hooks configured (SessionStart, UserPromptSubmit, PreToolUse[Read], PostToolUse, Stop, PreCompact, SessionEnd, Setup)');
  } else {
    console.log('  ⊘ Hooks skipped (--skills only)');
  }

  // 3. Skills
  if (doSkills) {
    for (const name of SKILL_NAMES) {
      const targetPath = installSkill(name, skillsDir);
      console.log(`  ✓ Skill '${name}' installed at ${targetPath}`);
    }
  } else {
    console.log('  ⊘ Skills skipped (--hooks only)');
  }

  // 4. Plugin manifest
  const manifestDir = installPluginManifest(join(sdkRoot, '.claude-plugin'), projectDir, '.claude-plugin');
  if (manifestDir) console.log(`  ✓ Plugin manifest installed at ${manifestDir}`);

  console.log('=== Claude Code installation complete ===');
  console.log('');
}

// ─── Codex installer ────────────────────────────────────────────────────

async function installCodex(projectDir: string, serverUrl: string, userId: string, skipMcp: boolean, installMode: string): Promise<void> {
  console.log('=== Installing for Codex ===');

  const sdkRoot = getSdkRoot();
  const mcpServerPath = join(sdkRoot, 'bin', 'memfuse-mcp.cjs');
  const hooksDir = join(sdkRoot, 'bin', 'hooks');
  const codexDir = join(projectDir, '.codex');
  const skillsDir = join(projectDir, '.codex', 'skills');
  const envVars = { MEMFUSE_USER_ID: userId, MEMFUSE_SERVER_URL: serverUrl };

  const doMcp = !skipMcp;
  const doHooks = installMode === 'full' || installMode === 'skills-hooks' || installMode === 'hooks-only';
  const doSkills = installMode === 'full' || installMode === 'skills-hooks' || installMode === 'skills-only';

  // 1. MCP server
  if (doMcp) {
    const cliAdded = tryMcpCliAdd(['codex', 'mcp', 'add'], 'memfuse', 'node', [mcpServerPath], envVars);
    if (cliAdded) {
      console.log('  ✓ MCP server added via `codex mcp add`');
    } else {
      mkdirSync(codexDir, { recursive: true });
      const mcpFile = join(codexDir, 'mcp.json');
      ensureJsonFile(mcpFile);
      mergeJson(mcpFile, { mcpServers: { memfuse: { command: 'node', args: [mcpServerPath], env: envVars } } });
      console.log(`  ✓ MCP server configured in ${mcpFile} (CLI unavailable)`);
    }
  } else {
    console.log('  ⊘ MCP registration skipped (--no-mcp)');
  }

  // 2. Hooks
  if (doHooks) {
    mkdirSync(codexDir, { recursive: true });
    const hooksFile = join(codexDir, 'hooks.json');
    const hookSpecs: CodexHookSpec[] = [
      {
        eventName: 'SessionStart',
        matcher: 'startup|resume|clear|compact',
        command: buildHookCommand(hooksDir, 'session-start.cjs', 'codex', envVars),
        timeout: 15,
      },
      {
        eventName: 'PostToolUse',
        matcher: 'Bash|Read|Edit|Write|MultiEdit|Glob|Grep|mcp__.*',
        command: buildHookCommand(hooksDir, 'post-tool-use.cjs', 'codex', envVars),
        timeout: 15,
      },
      {
        eventName: 'Stop',
        command: buildHookCommand(hooksDir, 'stop.cjs', 'codex', envVars),
        timeout: 10,
      },
    ];
    ensureJsonFile(hooksFile);
    mergeHooksConfig(hooksFile, {
      SessionStart: [{ matcher: hookSpecs[0].matcher, hooks: [{ type: 'command', command: hookSpecs[0].command, timeout: hookSpecs[0].timeout }] }],
      PostToolUse: [{ matcher: hookSpecs[1].matcher, hooks: [{ type: 'command', command: hookSpecs[1].command, timeout: hookSpecs[1].timeout }] }],
      Stop: [{ hooks: [{ type: 'command', command: hookSpecs[2].command, timeout: hookSpecs[2].timeout }] }],
    });
    const codexConfigFile = join(getCodexHome(), 'config.toml');
    writeCodexHookTrustConfig(codexConfigFile, hooksFile, hookSpecs);
    console.log('  ✓ Hooks configured (SessionStart, PostToolUse[Bash/Read/Edit/Write/Glob/Grep/MCP], Stop)');
    console.log(`  ✓ Codex hooks enabled and trusted in ${codexConfigFile}`);
    const hookSupport = codexCliHooksSupport();
    if (hookSupport === 'supported') {
      console.log('  ✓ Codex hooks support detected (`codex features list`: hooks=true)');
    } else if (hookSupport === 'unsupported') {
      console.log('  ⚠ Codex CLI does not report hooks support');
      console.log('    MemFuse still works in MCP + Skill mode — call resolve_context / store_observation explicitly.');
    } else {
      console.log('  ⚠ Could not detect Codex hooks support');
      console.log('    If this Codex CLI lacks hooks, MemFuse still works in MCP + Skill mode.');
    }
  } else {
    console.log('  ⊘ Hooks skipped (--skills only)');
  }

  // 3. Skills
  if (doSkills) {
    for (const name of SKILL_NAMES) {
      const targetPath = installSkill(name, skillsDir);
      console.log(`  ✓ Skill '${name}' installed at ${targetPath}`);
    }
  } else {
    console.log('  ⊘ Skills skipped (--hooks only)');
  }

  // 4. Plugin manifest
  const manifestDir = installPluginManifest(join(sdkRoot, '.codex-plugin'), projectDir, '.codex-plugin');
  if (manifestDir) console.log(`  ✓ Plugin manifest installed at ${manifestDir}`);

  console.log('=== Codex installation complete ===');
  console.log('');
}

// ─── Health check ────────────────────────────────────────────────────────

async function healthCheck(serverUrl: string): Promise<void> {
  console.log('=== Health Check ===');
  try {
    const result = await httpRequest(serverUrl, 'GET', '/health', null);
    if (result.statusCode >= 200 && result.statusCode < 400) {
      console.log('  ✓ MemFuse server online');
    } else {
      console.log('  ⚠ MemFuse server unhealthy — status', result.statusCode);
    }
  } catch {
    console.log('  ⚠ MemFuse server offline — will operate in degraded mode');
    console.log('    Start the service: memfuse service start');
    console.log('    Development server: ./run-server.sh');
  }
  console.log('=== Health check complete ===');
}
