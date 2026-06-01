#!/usr/bin/env node

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { createWriteStream } from 'node:fs';
import { access, appendFile, chmod, cp, mkdir, mkdtemp, readFile, realpath, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { homedir, tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const sdkRoot = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const repoRoot = resolve(fileURLToPath(new URL('../../..', import.meta.url)));
const fixturesRoot = join(sdkRoot, 'tests', 'e2e', 'fixtures');
const defaultTimeoutMs = 180_000;

function parseArgs(argv) {
  return {
    keepArtifacts: argv.includes('--keep-artifacts'),
    skipModel: process.env.MEMFUSE_E2E_SKIP_MODEL === '1' || argv.includes('--skip-model'),
    timeoutMs: Number(process.env.MEMFUSE_E2E_TIMEOUT_MS || defaultTimeoutMs),
    codexBin: process.env.MEMFUSE_E2E_CODEX_BIN || 'codex',
    sourceCodexHome: process.env.MEMFUSE_E2E_SOURCE_CODEX_HOME || join(homedir(), '.codex'),
    model: process.env.MEMFUSE_E2E_MODEL || '',
  };
}

function commandOk(command, args, options = {}) {
  const result = spawnSync(command, args, { encoding: 'utf8', ...options });
  return {
    ok: result.status === 0,
    stdout: result.stdout || '',
    stderr: result.stderr || '',
    status: result.status,
    signal: result.signal,
  };
}

async function pathExists(path) {
  try {
    await access(path);
    return true;
  } catch {
    return false;
  }
}

async function fetchJson(url, options = {}) {
  const response = await fetch(url, options);
  const text = await response.text();
  let body = {};
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    body = { text };
  }
  if (!response.ok) {
    throw new Error(`${options.method || 'GET'} ${url} failed ${response.status}: ${text}`);
  }
  return body;
}

async function getAvailablePort() {
  return new Promise((resolvePort, reject) => {
    const server = net.createServer();
    server.on('error', reject);
    server.listen(0, '127.0.0.1', () => {
      const address = server.address();
      const port = typeof address === 'object' && address ? address.port : 0;
      server.close(() => resolvePort(port));
    });
  });
}

async function waitForHealth(serverUrl, deadlineMs) {
  const deadline = Date.now() + deadlineMs;
  let lastError;
  while (Date.now() < deadline) {
    try {
      return await fetchJson(`${serverUrl}/health`);
    } catch (error) {
      lastError = error;
      await sleep(250);
    }
  }
  throw lastError || new Error('server did not become healthy');
}

function sleep(ms) {
  return new Promise((resolveSleep) => setTimeout(resolveSleep, ms));
}

async function timeRequest(name, fn, samples, report) {
  const durations = [];
  let lastResult;
  for (let i = 0; i < samples; i++) {
    const started = performance.now();
    lastResult = await fn(i);
    durations.push(performance.now() - started);
  }
  report.latency.direct_http[name] = summarizeDurations(durations);
  return lastResult;
}

function summarizeDurations(values) {
  const sorted = [...values].sort((a, b) => a - b);
  const pick = (p) => sorted[Math.min(sorted.length - 1, Math.floor((sorted.length - 1) * p))] || 0;
  return {
    count: sorted.length,
    min: roundMs(sorted[0] || 0),
    p50: roundMs(pick(0.50)),
    p95: roundMs(pick(0.95)),
    max: roundMs(sorted[sorted.length - 1] || 0),
  };
}

function roundMs(value) {
  return Math.round(value * 100) / 100;
}

const codexHookEventKeys = {
  SessionStart: 'session_start',
  PostToolUse: 'post_tool_use',
  Stop: 'stop',
};

const codexHookEventsWithMatchers = new Set(['SessionStart', 'PostToolUse']);

function canonicalJson(value) {
  if (Array.isArray(value)) return value.map(canonicalJson);
  if (value && typeof value === 'object') {
    return Object.fromEntries(
      Object.keys(value)
        .sort()
        .map((key) => [key, canonicalJson(value[key])]),
    );
  }
  return value;
}

function codexHookTrustHash(spec) {
  const identity = {
    event_name: codexHookEventKeys[spec.eventName],
    hooks: [{
      type: 'command',
      command: spec.command,
      timeout: Math.max(1, spec.timeout),
      async: false,
    }],
  };
  if (codexHookEventsWithMatchers.has(spec.eventName) && spec.matcher !== undefined) {
    identity.matcher = spec.matcher;
  }
  const serialized = JSON.stringify(canonicalJson(identity));
  return `sha256:${createHash('sha256').update(serialized).digest('hex')}`;
}

function tomlQuote(value) {
  return `"${value.replace(/\\/g, '\\\\').replace(/"/g, '\\"')}"`;
}

async function codexHookTrustToml(hooksFile, specs) {
  const canonicalHooksFile = await realpath(hooksFile);
  return specs.map((spec) => {
    const key = `${canonicalHooksFile}:${codexHookEventKeys[spec.eventName]}:0:0`;
    return `[hooks.state.${tomlQuote(key)}]\ntrusted_hash = ${tomlQuote(codexHookTrustHash(spec))}`;
  }).join('\n\n');
}

async function runCommand(command, args, options = {}) {
  const started = Date.now();
  const child = spawn(command, args, {
    cwd: options.cwd || repoRoot,
    env: options.env || process.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
  child.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
  if (options.input) child.stdin.end(options.input);
  else child.stdin.end();

  let killTimer = null;
  const timeout = options.timeoutMs
    ? setTimeout(() => {
      child.kill('SIGTERM');
      killTimer = setTimeout(() => child.kill('SIGKILL'), 5_000);
    }, options.timeoutMs)
    : null;

  const result = await new Promise((resolveResult) => {
    child.on('close', (code, signal) => resolveResult({ code, signal }));
  });
  if (timeout) clearTimeout(timeout);
  if (killTimer) clearTimeout(killTimer);

  return {
    command,
    args,
    stdout,
    stderr,
    code: result.code,
    signal: result.signal,
    duration_ms: Date.now() - started,
  };
}

async function startServer(runRoot, report) {
  const artifacts = join(runRoot, 'artifacts');
  const workspaceRoot = join(runRoot, 'memfuse-workspace');
  await mkdir(workspaceRoot, { recursive: true });
  await mkdir(artifacts, { recursive: true });

  const port = await getAvailablePort();
  const serverUrl = `http://127.0.0.1:${port}`;
  const logPath = join(artifacts, 'memfuse-server.log');
  const logStream = createWriteStream(logPath, { flags: 'a' });
  const env = {
    ...process.env,
    MEMFUSE_WORKSPACE_ROOT: workspaceRoot,
    MEMFUSE_BIND_ADDR: `127.0.0.1:${port}`,
    MEMFUSE_SERVER_URL: serverUrl,
    MEMFUSE_ACCOUNT_ID: 'e2e-account',
    MEMFUSE_USER_ID: 'e2e-user',
    MEMFUSE_AGENT_ID: 'codex-e2e-agent',
    MEMFUSE_SUMMARY_PROVIDER: 'deterministic',
    MEMFUSE_EMBEDDING_PROVIDER: 'deterministic',
    MEMFUSE_CHAT_PROVIDER: 'deterministic',
    RUST_LOG: process.env.RUST_LOG || 'mfs_server=info',
  };

  const child = spawn('cargo', ['run', '-p', 'mfs-server'], {
    cwd: repoRoot,
    env,
    stdio: ['ignore', 'pipe', 'pipe'],
  });
  child.stdout.pipe(logStream);
  child.stderr.pipe(logStream);

  try {
    const health = await waitForHealth(serverUrl, 60_000);
    report.server_url = serverUrl;
    report.memfuse_workspace = workspaceRoot;
    report.artifacts.memfuse_server_log = logPath;
    report.health = health;
    return { child, serverUrl, workspaceRoot, logPath };
  } catch (error) {
    child.kill('SIGTERM');
    throw error;
  }
}

async function stopServer(server) {
  if (!server?.child || server.child.killed) return;
  server.child.kill('SIGTERM');
  await new Promise((resolveStop) => {
    const timer = setTimeout(() => {
      if (!server.child.killed) server.child.kill('SIGKILL');
      resolveStop();
    }, 10_000);
    server.child.once('close', () => {
      clearTimeout(timer);
      resolveStop();
    });
  });
}

async function createSession(serverUrl, sessionId) {
  return fetchJson(`${serverUrl}/sessions`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ session_id: sessionId }),
  });
}

async function searchMemory(serverUrl, query, sessionId = 'e2e-search') {
  return fetchJson(`${serverUrl}/v1/memory:search`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ query, user_id: 'e2e-user', session_id: sessionId, limit: 10 }),
  });
}

async function sessionContext(serverUrl, sessionId) {
  return fetchJson(`${serverUrl}/sessions/${encodeURIComponent(sessionId)}/context?token_budget=4000`);
}

function searchResultsContain(searchResult, needle) {
  const results = Array.isArray(searchResult?.results) ? searchResult.results : [];
  return results.some((item) => JSON.stringify(item).includes(needle));
}

function addScenario(report, scenario) {
  report.scenarios.push({
    status: 'passed',
    evidence: {},
    ...scenario,
  });
}

function failScenario(report, id, name, failureClass, evidence = {}) {
  addScenario(report, { id, name, status: 'failed', failure_class: failureClass, evidence });
}

async function runDirectDiagnostics(serverUrl, report, env) {
  const name = 'D1';
  const scenario = { id: name, name: 'Direct HTTP and Hook Diagnostics', status: 'passed', evidence: {} };
  try {
    await createSession(serverUrl, 'direct-diagnostics');
    await timeRequest('store_observation', (i) => fetchJson(`${serverUrl}/sessions/direct-diagnostics/observations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'ManualNote',
        tool_input: '',
        tool_output: '',
        content: `DIRECT_DIAGNOSTIC_SENTINEL ${i} stored through real HTTP`,
        platform: 'e2e',
      }),
    }), 5, report);

    await timeRequest('resolve_context', () => fetchJson(`${serverUrl}/context/resolve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        query: 'DIRECT_DIAGNOSTIC_SENTINEL',
        session_id: 'direct-diagnostics',
        user_id: 'e2e-user',
        token_budget: 1200,
      }),
    }), 5, report);

    await timeRequest('search_memories', () => searchMemory(serverUrl, 'DIRECT_DIAGNOSTIC_SENTINEL', 'direct-diagnostics'), 5, report);
    await timeRequest('list_facts', () => fetchJson(`${serverUrl}/facts?user_id=e2e-user`), 5, report);

    await createSession(serverUrl, 'direct-hook-session');
    const postToolPayload = {
      hook_event_name: 'PostToolUse',
      session_id: 'direct-hook-session',
      tool_name: 'Bash',
      tool_input: { command: 'printf DIRECT_HOOK_SENTINEL' },
      tool_response: 'DIRECT_HOOK_SENTINEL',
    };
    const hookEnv = { ...env, MEMFUSE_SESSION_ID: 'direct-hook-session' };
    const postTool = await runCommand(process.execPath, [join(sdkRoot, 'bin', 'hooks', 'post-tool-use.cjs')], {
      cwd: sdkRoot,
      env: hookEnv,
      input: JSON.stringify(postToolPayload),
      timeoutMs: 20_000,
    });
    assert.equal(postTool.code, 0, `PostToolUse hook failed: ${postTool.stderr}`);

    const stopPayload = {
      hook_event_name: 'Stop',
      session_id: 'direct-hook-session',
      last_assistant_message: 'Completed direct hook diagnostic. The DIRECT_HOOK_SENTINEL command output was observed and should be remembered for this test run.',
      reason: 'e2e-direct-hook',
      source: 'codex-cli',
    };
    const stop = await runCommand(process.execPath, [join(sdkRoot, 'bin', 'hooks', 'stop.cjs')], {
      cwd: sdkRoot,
      env: hookEnv,
      input: JSON.stringify(stopPayload),
      timeoutMs: 20_000,
    });
    assert.equal(stop.code, 0, `Stop hook failed: ${stop.stderr}`);

    const sessionStartPayload = {
      hook_event_name: 'SessionStart',
      source: 'codex-cli',
      transcript_path: join(sdkRoot, 'tests', 'e2e', 'fake-transcript.jsonl'),
      prompt: 'DIRECT_HOOK_SENTINEL',
    };
    const sessionStart = await runCommand(process.execPath, [join(sdkRoot, 'bin', 'hooks', 'session-start.cjs')], {
      cwd: sdkRoot,
      env: hookEnv,
      input: JSON.stringify(sessionStartPayload),
      timeoutMs: 20_000,
    });
    assert.equal(sessionStart.code, 0, `SessionStart hook failed: ${sessionStart.stderr}`);
    const parsedStart = JSON.parse(sessionStart.stdout || '{}');
    assert.equal(parsedStart.hookSpecificOutput?.hookEventName, 'SessionStart');

    const hookSessionContext = await sessionContext(serverUrl, 'direct-hook-session').catch((error) => ({ error: error.message }));
    const found = JSON.stringify(hookSessionContext).includes('DIRECT_HOOK_SENTINEL');
    scenario.evidence = {
      post_tool_duration_ms: postTool.duration_ms,
      stop_duration_ms: stop.duration_ms,
      session_start_duration_ms: sessionStart.duration_ms,
      session_start_output_is_codex_json: true,
      session_context_contains_direct_hook_sentinel: found,
    };
    if (!found) {
      scenario.status = 'failed';
      scenario.failure_class = 'hook_capture_missing';
    }
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'direct_diagnostics_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
  }
  addScenario(report, scenario);
}

async function setupIsolatedCodex(runRoot, serverUrl, report, options) {
  const codexHome = join(runRoot, 'codex-home');
  const projectRoot = join(runRoot, 'project');
  const artifacts = join(runRoot, 'artifacts');
  const hookRoot = join(runRoot, 'hooks');
  const binRoot = join(runRoot, 'bin');
  const hookTiming = join(artifacts, 'hook-timing.jsonl');

  await mkdir(join(codexHome, 'skills'), { recursive: true });
  await mkdir(projectRoot, { recursive: true });
  await mkdir(hookRoot, { recursive: true });
  await mkdir(binRoot, { recursive: true });
  await cp(join(fixturesRoot, 'AGENTS.md'), join(projectRoot, 'AGENTS.md'));
  await mkdir(join(projectRoot, 'fixtures'), { recursive: true });
  await cp(join(fixturesRoot, 'agent-task.md'), join(projectRoot, 'fixtures', 'agent-task.md'));

  const sourceAuth = join(options.sourceCodexHome, 'auth.json');
  if (await pathExists(sourceAuth)) {
    await cp(sourceAuth, join(codexHome, 'auth.json'));
    report.codex_auth = { source: 'copied-auth-json', scrubbed_from_artifacts: true };
  } else {
    report.codex_auth = { source: 'missing' };
  }

  const memfuseShim = join(binRoot, 'memfuse');
  await writeFile(memfuseShim, `#!/usr/bin/env bash\nexec "${process.execPath}" "${join(sdkRoot, 'bin', 'memfuse.cjs')}" "$@"\n`);
  await chmod(memfuseShim, 0o755);

  const wrapperPath = join(hookRoot, 'hook-wrapper.mjs');
  await writeFile(wrapperPath, `#!/usr/bin/env node
import { spawn } from 'node:child_process';
import { appendFileSync } from 'node:fs';

const event = process.argv[2];
const target = process.argv[3];
const log = process.env.MEMFUSE_E2E_HOOK_TIMING;
const started = Date.now();
const child = spawn(process.execPath, [target], { stdio: ['pipe', 'pipe', 'pipe'], env: process.env });
process.stdin.pipe(child.stdin);
let stderr = '';
child.stdout.pipe(process.stdout);
child.stderr.on('data', (chunk) => {
  const text = chunk.toString();
  stderr += text;
  process.stderr.write(text);
});
child.on('close', (code) => {
  const ended = Date.now();
  appendFileSync(log, JSON.stringify({
    event,
    start_ms: started,
    end_ms: ended,
    duration_ms: ended - started,
    exit_code: code,
    stderr_tail: stderr.slice(-1000),
  }) + '\\n');
  process.exit(code ?? 0);
});
`);
  await chmod(wrapperPath, 0o755);

  const hookCommand = (event, file) => `${process.execPath} ${JSON.stringify(wrapperPath)} ${event} ${JSON.stringify(join(sdkRoot, 'bin', 'hooks', file))}`;
  const hookSpecs = [
    {
      eventName: 'SessionStart',
      matcher: 'startup|resume|clear|compact',
      command: hookCommand('SessionStart', 'session-start.cjs'),
      timeout: 15,
    },
    {
      eventName: 'PostToolUse',
      matcher: 'Bash|Read|Edit|Write|MultiEdit|Glob|Grep|mcp__.*',
      command: hookCommand('PostToolUse', 'post-tool-use.cjs'),
      timeout: 15,
    },
    {
      eventName: 'Stop',
      command: hookCommand('Stop', 'stop.cjs'),
      timeout: 20,
    },
  ];
  const hooks = {
    hooks: {
      SessionStart: [{ matcher: hookSpecs[0].matcher, hooks: [{ type: 'command', command: hookSpecs[0].command, timeout: hookSpecs[0].timeout }] }],
      PostToolUse: [{ matcher: hookSpecs[1].matcher, hooks: [{ type: 'command', command: hookSpecs[1].command, timeout: hookSpecs[1].timeout }] }],
      Stop: [{ hooks: [{ type: 'command', command: hookSpecs[2].command, timeout: hookSpecs[2].timeout }] }],
    },
  };
  const hooksFile = join(codexHome, 'hooks.json');
  await writeFile(hooksFile, JSON.stringify(hooks, null, 2) + '\n');
  await mkdir(join(projectRoot, '.codex'), { recursive: true });
  const configSeed = await buildCodexConfigSeed(options.sourceCodexHome);
  const hookTrust = await codexHookTrustToml(hooksFile, hookSpecs);
  await writeFile(join(codexHome, 'config.toml'), `${configSeed}

[features]
hooks = true

${hookTrust}

[projects.${JSON.stringify(projectRoot)}]
trust_level = "trusted"
`);

  const env = {
    ...process.env,
    CODEX_HOME: codexHome,
    MEMFUSE_SERVER_URL: serverUrl,
    MEMFUSE_USER_ID: 'e2e-user',
    MEMFUSE_E2E_HOOK_TIMING: hookTiming,
    PATH: `${binRoot}:${process.env.PATH || ''}`,
  };

  report.codex_home = codexHome;
  report.project_root = projectRoot;
  report.artifacts.hook_timing = hookTiming;
  report.artifacts.codex_events = join(artifacts, 'codex-events.jsonl');

  return { codexHome, projectRoot, artifacts, hookTiming, env };
}

async function buildCodexConfigSeed(sourceCodexHome) {
  const configPath = join(sourceCodexHome, 'config.toml');
  if (!(await pathExists(configPath))) return '';
  const source = await readFile(configPath, 'utf8');
  const providerMatch = source.match(/^model_provider\s*=\s*"([^"]+)"/m);
  const provider = providerMatch?.[1];
  const seed = [];
  const topLevelKeys = [
    'model_provider',
    'model',
    'model_context_window',
    'model_auto_compact_token_limit',
    'service_tier',
    'model_reasoning_effort',
    'disable_response_storage',
  ];
  for (const key of topLevelKeys) {
    const match = source.match(new RegExp(`^${key}\\s*=\\s*.+$`, 'm'));
    if (match) seed.push(match[0]);
  }
  if (provider) {
    const escapedProvider = provider.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
    const blockMatch = source.match(new RegExp(`\\[model_providers\\.${escapedProvider}\\][\\s\\S]*?(?=\\n\\[|$)`));
    if (blockMatch) seed.push(blockMatch[0].trimEnd());
  }
  return seed.join('\n');
}

async function installSkillTo(targetRoot) {
  const src = join(sdkRoot, 'dist', 'skills', 'memfuse');
  if (!(await pathExists(src))) {
    throw new Error(`built MemFuse skill not found at ${src}; run npm run build first`);
  }
  const destDir = join(targetRoot, 'skills', 'memfuse');
  await rm(destDir, { recursive: true, force: true });
  await mkdir(join(targetRoot, 'skills'), { recursive: true });
  await cp(src, destDir, { recursive: true });
}

async function runCodexExec(codex, prompt, sessionId, options, extraArgs = []) {
  const outputFile = join(codex.artifacts, `last-message-${Date.now()}-${Math.random().toString(16).slice(2)}.md`);
  const args = [
    'exec',
    '--json',
    '--output-last-message',
    outputFile,
    '--skip-git-repo-check',
    '--dangerously-bypass-approvals-and-sandbox',
    '-C',
    codex.projectRoot,
    ...extraArgs,
  ];
  if (options.model) args.push('-m', options.model);
  args.push(prompt);

  const env = {
    ...codex.env,
    MEMFUSE_SESSION_ID: sessionId,
    MEMFUSE_THREAD_ID: sessionId,
  };
  const result = await runCommand(options.codexBin, args, {
    cwd: codex.projectRoot,
    env,
    timeoutMs: options.timeoutMs,
  });
  await appendFile(join(codex.artifacts, 'codex-events.jsonl'), result.stdout);
  if (result.stderr) await appendFile(join(codex.artifacts, 'codex-stderr.log'), result.stderr);

  let finalMessage = '';
  if (await pathExists(outputFile)) finalMessage = await readFile(outputFile, 'utf8');
  const events = result.stdout
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      try { return JSON.parse(line); } catch { return { raw: line }; }
    });
  return { ...result, events, finalMessage, outputFile };
}

async function runCodexResume(codex, threadId, prompt, sessionId, options) {
  const outputFile = join(codex.artifacts, `last-message-resume-${Date.now()}-${Math.random().toString(16).slice(2)}.md`);
  const args = [
    'exec',
    '--json',
    '--output-last-message',
    outputFile,
    '--skip-git-repo-check',
    '--dangerously-bypass-approvals-and-sandbox',
    '-C',
    codex.projectRoot,
    'resume',
    threadId,
  ];
  if (options.model) args.push('-m', options.model);
  args.push(prompt);
  const env = {
    ...codex.env,
    MEMFUSE_SESSION_ID: sessionId,
    MEMFUSE_THREAD_ID: sessionId,
  };
  const result = await runCommand(options.codexBin, args, {
    cwd: codex.projectRoot,
    env,
    timeoutMs: options.timeoutMs,
  });
  await appendFile(join(codex.artifacts, 'codex-events.jsonl'), result.stdout);
  if (result.stderr) await appendFile(join(codex.artifacts, 'codex-stderr.log'), result.stderr);
  let finalMessage = '';
  if (await pathExists(outputFile)) finalMessage = await readFile(outputFile, 'utf8');
  const events = result.stdout
    .split('\n')
    .filter(Boolean)
    .map((line) => {
      try { return JSON.parse(line); } catch { return { raw: line }; }
    });
  return { ...result, events, finalMessage, outputFile };
}

async function listCodexHooks(codex, options) {
  const child = spawn(options.codexBin, ['app-server', '--listen', 'stdio://'], {
    cwd: codex.projectRoot,
    env: codex.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  let stdout = '';
  let stderr = '';
  let buffer = '';
  const waiters = new Map();
  let initialized = false;

  const waitForResponse = (id, timeoutMs) => new Promise((resolveWait, rejectWait) => {
    const timer = setTimeout(() => {
      waiters.delete(id);
      rejectWait(new Error(`codex app-server response ${id} timed out`));
    }, timeoutMs);
    waiters.set(id, {
      resolve: (value) => {
        clearTimeout(timer);
        resolveWait(value);
      },
      reject: (error) => {
        clearTimeout(timer);
        rejectWait(error);
      },
    });
  });

  function send(message) {
    child.stdin.write(`${JSON.stringify(message)}\n`);
  }

  function handleMessage(message) {
    if (!initialized && message.id === 1) {
      initialized = true;
      send({ method: 'initialized' });
      send({ id: 2, method: 'hooks/list', params: { cwds: [codex.projectRoot] } });
    }
    if (message.id !== undefined && waiters.has(message.id)) {
      const waiter = waiters.get(message.id);
      waiters.delete(message.id);
      if (message.error) waiter.reject(new Error(JSON.stringify(message.error)));
      else waiter.resolve(message.result);
    }
  }

  child.stdout.on('data', (chunk) => {
    const text = chunk.toString();
    stdout += text;
    buffer += text;
    for (;;) {
      const index = buffer.indexOf('\n');
      if (index === -1) break;
      const line = buffer.slice(0, index).trim();
      buffer = buffer.slice(index + 1);
      if (!line) continue;
      try {
        handleMessage(JSON.parse(line));
      } catch {
        // Keep malformed app-server output in stdout for diagnostics.
      }
    }
  });
  child.stderr.on('data', (chunk) => { stderr += chunk.toString(); });

  const initResponse = waitForResponse(1, 10_000);
  const hooksResponse = waitForResponse(2, 10_000);

  send({
    id: 1,
    method: 'initialize',
    params: {
      clientInfo: { name: 'memfuse-codex-e2e', title: null, version: '0' },
      capabilities: null,
    },
  });

  try {
    await initResponse;
    const result = await hooksResponse;
    return { result, stdout, stderr };
  } finally {
    child.stdin.end();
    child.kill('SIGTERM');
    await new Promise((resolveClose) => {
      const timer = setTimeout(() => {
        if (!child.killed) child.kill('SIGKILL');
        resolveClose();
      }, 2_000);
      child.once('close', () => {
        clearTimeout(timer);
        resolveClose();
      });
    });
  }
}

async function runCodexHookDiscoveryDiagnostics(report, codex, options) {
  const scenario = { id: 'D2', name: 'Codex Hook Discovery Config', status: 'passed', evidence: {} };
  try {
    const listed = await listCodexHooks(codex, options);
    const entries = Array.isArray(listed.result?.data) ? listed.result.data : [];
    const hooks = entries.flatMap((entry) => Array.isArray(entry.hooks) ? entry.hooks : []);
    const byEvent = new Map(hooks.map((hook) => [hook.eventName, hook]));
    const requiredEvents = ['sessionStart', 'postToolUse', 'stop'];
    const missing = requiredEvents.filter((event) => !byEvent.has(event));
    const untrusted = requiredEvents.filter((event) => byEvent.get(event)?.trustStatus !== 'trusted');
    scenario.evidence = {
      hook_count: hooks.length,
      events: hooks.map((hook) => ({
        event: hook.eventName,
        trust_status: hook.trustStatus,
        source: hook.source,
        matcher: hook.matcher,
      })),
      warnings: entries.flatMap((entry) => entry.warnings || []),
      errors: entries.flatMap((entry) => entry.errors || []),
    };
    if (missing.length > 0 || untrusted.length > 0) {
      scenario.status = 'failed';
      scenario.failure_class = missing.length > 0 ? 'codex_hooks_not_discovered' : 'codex_hooks_untrusted';
      scenario.evidence.missing_events = missing;
      scenario.evidence.untrusted_events = untrusted;
    }
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'codex_hook_discovery_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
  }
  addScenario(report, scenario);
}

function threadIdFrom(events) {
  const started = events.find((event) => event.type === 'thread.started' && event.thread_id);
  return started?.thread_id || '';
}

function eventText(events) {
  return events.map((event) => JSON.stringify(event)).join('\n');
}

function parseFeatureFlags(featuresText) {
  const features = new Map();
  for (const line of featuresText.split('\n')) {
    const parts = line.trim().split(/\s+/);
    if (parts.length >= 3) features.set(parts[0], parts[2] === 'true');
  }
  return features;
}

function codexSupportsHooks(featuresText) {
  const features = parseFeatureFlags(featuresText);
  return features.get('codex_hooks') === true || features.get('hooks') === true;
}

function codexSkillFeatureArgs(featuresText) {
  const features = parseFeatureFlags(featuresText);
  return features.has('skills') ? ['--enable', 'skills'] : [];
}

function hookUnsupportedScenario(id, name, report, featuresText) {
  addScenario(report, {
    id,
    name,
    status: 'skipped',
    failure_class: 'codex_hooks_unsupported_by_cli',
    evidence: {
      codex_version: report.codex_version,
      reason: 'Codex CLI feature list does not expose codex_hooks/hooks, so exec cannot invoke lifecycle hooks in this environment.',
      features: featuresText,
    },
  });
}

async function runCodexScenarios(serverUrl, report, codex, options) {
  if (options.skipModel) {
    addScenario(report, {
      id: 'MODEL',
      name: 'Real Codex Model Scenarios',
      status: 'skipped',
      evidence: { reason: '--skip-model enabled' },
    });
    return;
  }

  const smoke = await runCodexExec(codex, 'Say exactly CODEX_E2E_SMOKE_OK and do not run tools.', 'codex-smoke', options);
  if (smoke.code !== 0 || !smoke.finalMessage.includes('CODEX_E2E_SMOKE_OK')) {
    failScenario(report, 'S0', 'Codex Smoke', 'codex_unavailable', {
      code: smoke.code,
      stderr_tail: smoke.stderr.slice(-1000),
      final_message: smoke.finalMessage,
    });
    return;
  }
  addScenario(report, { id: 'S0', name: 'Codex Smoke', evidence: { duration_ms: smoke.duration_ms } });

  await installSkillTo(codex.codexHome);
  await installSkillTo(join(codex.projectRoot, '.codex'));
  const visible = await runCodexExec(
    codex,
    'If a memfuse skill is available, answer MEMFUSE_SKILL_VISIBLE. Otherwise answer MEMFUSE_SKILL_MISSING. Do not run shell commands.',
    'codex-skill-visible',
    options,
    codexSkillFeatureArgs(report.codex_features || ''),
  );
  addScenario(report, {
    id: 'S1',
    name: 'Codex Home Skill Visibility',
    status: visible.finalMessage.includes('MEMFUSE_SKILL_VISIBLE') ? 'passed' : 'failed',
    failure_class: visible.finalMessage.includes('MEMFUSE_SKILL_VISIBLE') ? undefined : 'skill_not_discovered_codex_home_layout',
    evidence: { final_message: visible.finalMessage, duration_ms: visible.duration_ms },
  });

  await createSession(serverUrl, 'codex-active-workflow');
  const active = await runCodexExec(
    codex,
    'Use the MemFuse skill if available. Start by checking memory for this task. Then inspect fixtures/agent-task.md and save the key decision that E2E_TOKEN_ALPHA maps to the blue deployment lane. Do not edit files.',
    'codex-active-workflow',
    options,
    codexSkillFeatureArgs(report.codex_features || ''),
  );
  const activeText = `${eventText(active.events)}\n${active.finalMessage}`;
  const activeSearch = await searchMemory(serverUrl, 'E2E_TOKEN_ALPHA blue deployment lane', 'codex-active-workflow');
  const activeFound = searchResultsContain(activeSearch, 'E2E_TOKEN_ALPHA');
  const activeSessionContext = await sessionContext(serverUrl, 'codex-active-workflow').catch((error) => ({ error: error.message }));
  const activeStored = activeText.includes('Stored codex-active-workflow')
    || JSON.stringify(activeSessionContext).includes('E2E_TOKEN_ALPHA');
  addScenario(report, {
    id: 'S2',
    name: 'Active Skill Workflow',
    status: activeText.includes('memfuse') && activeStored ? 'passed' : 'failed',
    failure_class: activeText.includes('memfuse') ? (activeStored ? undefined : 'observation_not_persisted') : 'skill_not_used',
    evidence: {
      codex_exit_code: active.code,
      used_memfuse_command: activeText.includes('memfuse'),
      observation_stored: activeStored,
      memory_search_contains_sentinel: activeFound,
      final_message: active.finalMessage,
    },
  });

  await createSession(serverUrl, 'codex-passive-hook');
  if (!report.codex_capabilities?.hooks) {
    hookUnsupportedScenario('S3', 'Passive Hook Capture', report, report.codex_features || '');
  } else {
    const passive = await runCodexExec(
      codex,
      'Run this exact shell command: printf E2E_HOOK_SENTINEL Then summarize what happened. Do not call memfuse manually.',
      'codex-passive-hook',
      options,
    );
    await sleep(1000);
    const hookTimingText = await readFile(codex.hookTiming, 'utf8').catch(() => '');
    const passiveSearch = await searchMemory(serverUrl, 'E2E_HOOK_SENTINEL', 'codex-passive-hook');
    const passiveFound = searchResultsContain(passiveSearch, 'E2E_HOOK_SENTINEL');
    const passiveSessionContext = await sessionContext(serverUrl, 'codex-passive-hook').catch((error) => ({ error: error.message }));
    const passiveStored = JSON.stringify(passiveSessionContext).includes('E2E_HOOK_SENTINEL');
    addScenario(report, {
      id: 'S3',
      name: 'Passive Hook Capture',
      status: hookTimingText.includes('PostToolUse') && hookTimingText.includes('Stop') && passiveStored ? 'passed' : 'failed',
      failure_class: !hookTimingText.includes('PostToolUse')
        ? 'post_tool_hook_not_invoked'
        : !hookTimingText.includes('Stop')
          ? 'stop_hook_not_invoked'
          : passiveStored ? undefined : 'hook_capture_missing',
      evidence: {
        codex_exit_code: passive.code,
        post_tool_hook_seen: hookTimingText.includes('PostToolUse'),
        stop_hook_seen: hookTimingText.includes('Stop'),
        session_context_contains_sentinel: passiveStored,
        memory_search_contains_sentinel: passiveFound,
        final_message: passive.finalMessage,
      },
    });
  }

  const s4 = {
    id: 'S4',
    name: 'Cross-Session Memory Injection',
    status: 'passed',
    evidence: {},
  };
  await createSession(serverUrl, 'codex-cross-session-a');
  await fetchJson(`${serverUrl}/sessions/codex-cross-session-a/observations`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      tool_name: 'ManualNote',
      tool_input: '',
      tool_output: '',
      content: 'AURORA-917 means use SQLite WAL for the replay cache.',
      platform: 'e2e',
    }),
  });
  await fetchJson(`${serverUrl}/sessions/codex-cross-session-a/commit`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ user_id: 'e2e-user', reason: 'e2e-cross-session' }),
  });
  let observationContextContains = false;
  try {
    await waitForContext(serverUrl, 'AURORA-917', 'codex-cross-session-b', 20_000);
    observationContextContains = true;
  } catch {
    observationContextContains = false;
  }

  await fetchJson(`${serverUrl}/facts`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({
      id: `fact_aurora_${Date.now()}`,
      subject: 'AURORA-917',
      predicate: 'procedure.replay_cache',
      display_value: 'AURORA-917 means use SQLite WAL for the replay cache.',
      confidence: 0.99,
      agent_id: 'codex-e2e-agent',
    }),
  });
  const factContext = await waitForContext(serverUrl, 'AURORA-917', 'codex-cross-session-b', 20_000);
  const factContextContains = JSON.stringify(factContext).includes('AURORA-917');
  const context = await fetchJson(`${serverUrl}/context/resolve`, {
    method: 'POST',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify({ query: 'AURORA-917', user_id: 'e2e-user', session_id: 'codex-cross-session-b', token_budget: 1500 }),
  });
  const contextContains = JSON.stringify(context).includes('AURORA-917');
  if (!report.codex_capabilities?.hooks) {
    s4.status = contextContains ? 'skipped' : 'failed';
    s4.failure_class = contextContains ? 'codex_hooks_unsupported_by_cli' : 'context_resolve_misses_fact_sentinel';
    s4.evidence = {
      observation_commit_context_contains_sentinel: observationContextContains,
      fact_context_contains_sentinel: factContextContains,
      final_context_resolve_contains_sentinel: contextContains,
      codex_version: report.codex_version,
      reason: contextContains
        ? 'Cross-session injection depends on Codex SessionStart hooks, but this Codex CLI does not expose codex_hooks/hooks.'
        : 'MemFuse context resolver did not return the direct fact sentinel.',
      features: report.codex_features || '',
    };
  } else {
    const cross = await runCodexExec(
      codex,
      'Based only on any memory context injected at startup, what does AURORA-917 mean? If there is no injected memory, say NO_MEMORY. Do not search the web.',
      'codex-cross-session-b',
      options,
    );
    const s4HookTimingText = await readFile(codex.hookTiming, 'utf8').catch(() => '');
    const sessionStartHookSeen = s4HookTimingText.includes('SessionStart');
    s4.status = contextContains && cross.finalMessage.includes('SQLite WAL') ? 'passed' : 'failed';
    s4.failure_class = !contextContains
      ? 'context_resolve_misses_fact_sentinel'
      : cross.finalMessage.includes('SQLite WAL')
        ? undefined
        : sessionStartHookSeen
          ? 'model_ignored_injected_memory'
          : 'session_start_injection_missing';
    s4.evidence = {
      observation_commit_context_contains_sentinel: observationContextContains,
      fact_context_contains_sentinel: factContextContains,
      final_context_resolve_contains_sentinel: contextContains,
      session_start_hook_seen: sessionStartHookSeen,
      final_answer_contains_sqlite_wal: cross.finalMessage.includes('SQLite WAL'),
      final_message: cross.finalMessage,
    };
  }
  addScenario(report, s4);

  const resumeBase = await runCodexExec(
    codex,
    'Remember this phrase for the current conversation only: RESUME_BASE_OK. Then answer BASE_READY.',
    'codex-resume-base',
    options,
  );
  const resumeThreadId = threadIdFrom(resumeBase.events);
  let resumeResult = { code: 1, finalMessage: '', stderr: 'missing thread id' };
  if (resumeThreadId) {
    resumeResult = await runCodexResume(
      codex,
      resumeThreadId,
      'Continue this same conversation. What phrase did I ask you to remember? Answer with only that phrase.',
      'codex-resume-followup',
      options,
    );
  }
  addScenario(report, {
    id: 'S5',
    name: 'Codex Resume Multi-Turn Continuity',
    status: resumeResult.code === 0 && resumeResult.finalMessage.includes('RESUME_BASE_OK') ? 'passed' : 'failed',
    failure_class: !resumeThreadId
      ? 'resume_thread_id_missing'
      : resumeResult.code !== 0
        ? 'resume_invocation_failed'
        : resumeResult.finalMessage.includes('RESUME_BASE_OK')
          ? undefined
          : 'resume_context_missing',
    evidence: {
      base_exit_code: resumeBase.code,
      resume_thread_id_present: Boolean(resumeThreadId),
      resume_exit_code: resumeResult.code,
      final_message: resumeResult.finalMessage,
      stderr_tail: resumeResult.stderr?.slice?.(-1000) || '',
    },
  });
}

async function waitForContext(serverUrl, sentinel, sessionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const context = await fetchJson(`${serverUrl}/context/resolve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ query: sentinel, user_id: 'e2e-user', session_id: sessionId, token_budget: 1500 }),
    });
    if (JSON.stringify(context).includes(sentinel)) return context;
    await sleep(500);
  }
  throw new Error(`context did not include ${sentinel} within ${timeoutMs}ms`);
}

async function writeFailures(report) {
  const failures = report.scenarios.filter((scenario) => scenario.status === 'failed');
  if (failures.length === 0) return;
  const lines = ['# Codex E2E Memory Failures', ''];
  for (const failure of failures) {
    lines.push(`## ${failure.id}: ${failure.name}`);
    lines.push('');
    lines.push(`- Failure class: ${failure.failure_class || 'unknown'}`);
    lines.push(`- Evidence: \`${JSON.stringify(failure.evidence || {})}\``);
    lines.push('');
  }
  await writeFile(join(report.artifacts.root, 'artifacts', 'failures.md'), lines.join('\n'));
}

async function writeLimitations(report) {
  const limitations = report.scenarios.filter((scenario) => scenario.status === 'skipped' && scenario.failure_class);
  if (limitations.length === 0) return;
  const lines = ['# Codex E2E Memory Limitations', ''];
  for (const limitation of limitations) {
    lines.push(`## ${limitation.id}: ${limitation.name}`);
    lines.push('');
    lines.push(`- Class: ${limitation.failure_class}`);
    lines.push(`- Evidence: \`${JSON.stringify(limitation.evidence || {})}\``);
    lines.push('');
  }
  await writeFile(join(report.artifacts.root, 'artifacts', 'limitations.md'), lines.join('\n'));
}

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const runRoot = await mkdtemp(join(tmpdir(), 'memfuse-codex-e2e-'));
  await mkdir(join(runRoot, 'artifacts'), { recursive: true });
  const report = {
    run_id: new Date().toISOString().replace(/[-:.]/g, ''),
    status: 'running',
    scenarios: [],
    latency: { direct_http: {} },
    artifacts: { root: runRoot },
  };
  const reportPath = join(runRoot, 'artifacts', 'report.json');
  let server;

  try {
    const codexVersion = commandOk(options.codexBin, ['--version']);
    assert.equal(codexVersion.ok, true, `codex not available: ${codexVersion.stderr}`);
    report.codex_version = codexVersion.stdout.trim();

    const buildOutput = commandOk('npm', ['run', 'build', '--silent'], { cwd: sdkRoot });
    assert.equal(buildOutput.ok, true, `sdk build failed: ${buildOutput.stderr}`);

    server = await startServer(runRoot, report);
    const env = {
      ...process.env,
      MEMFUSE_SERVER_URL: server.serverUrl,
      MEMFUSE_USER_ID: 'e2e-user',
      MEMFUSE_SUMMARY_PROVIDER: 'deterministic',
      MEMFUSE_EMBEDDING_PROVIDER: 'deterministic',
    };
    await runDirectDiagnostics(server.serverUrl, report, env);

    const codex = await setupIsolatedCodex(runRoot, server.serverUrl, report, options);
    await runCodexHookDiscoveryDiagnostics(report, codex, options);
    const features = commandOk(options.codexBin, ['features', 'list'], { env: codex.env });
    report.codex_features = features.stdout;
    report.codex_capabilities = {
      hooks: codexSupportsHooks(features.stdout),
      skills: parseFeatureFlags(features.stdout).get('skills') === true,
    };
    await runCodexScenarios(server.serverUrl, report, codex, options);

    report.status = report.scenarios.some((scenario) => scenario.status === 'failed') ? 'failed' : 'passed';
  } catch (error) {
    report.status = 'failed';
    report.error = error instanceof Error ? error.stack || error.message : String(error);
  } finally {
    if (report.codex_home) {
      await rm(join(report.codex_home, 'auth.json'), { force: true });
      await rm(join(report.codex_home, 'credentials.json'), { force: true });
    }
    await stopServer(server);
    await writeFailures(report);
    await writeLimitations(report);
    await writeFile(reportPath, JSON.stringify(report, null, 2) + '\n');
    console.log(`Report: ${reportPath}`);
    console.log(`Artifacts: ${runRoot}`);
    if (!options.keepArtifacts && report.status === 'passed') {
      await rm(runRoot, { recursive: true, force: true });
    }
  }

  if (report.status === 'failed') {
    process.exit(1);
  }
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
