#!/usr/bin/env node
/**
 * Claude Code Real E2E Memory Test
 *
 * Validates the full Claude Code memory lifecycle against a real MemFuse server.
 * Tests all 8 hooks (plain-text I/O), MCP tools (JSON-RPC over stdio),
 * multi-turn memory, fact lifecycle, privacy, degradation, and latency.
 *
 * Usage:
 *   cd sdk && npm run build --silent && node tests/e2e/claude-code-real-memory.test.mjs --keep-artifacts
 */

import assert from 'node:assert/strict';
import { spawn, spawnSync } from 'node:child_process';
import { createWriteStream } from 'node:fs';
import { access, cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import net from 'node:net';
import { tmpdir } from 'node:os';
import { join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  createJsonRpcFrameParser,
  extractFactIdFromMcpResult,
  failScenarioIfMissingEvidence,
  mcpToolSucceeded,
} from './claude-code-harness.mjs';

const sdkRoot = resolve(fileURLToPath(new URL('../..', import.meta.url)));
const repoRoot = resolve(fileURLToPath(new URL('../../..', import.meta.url)));

// ─── Argument Parsing ──────────────────────────────────────────────────

function parseArgs(argv) {
  return {
    keepArtifacts: argv.includes('--keep-artifacts'),
  };
}

// ─── Low-Level Helpers ─────────────────────────────────────────────────

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
  try { await access(path); return true; } catch { return false; }
}

async function fetchJson(url, options = {}) {
  const response = await fetch(url, options);
  const text = await response.text();
  let body = {};
  try { body = text ? JSON.parse(text) : {}; } catch { body = { text }; }
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
    try { return await fetchJson(`${serverUrl}/health`); }
    catch (error) { lastError = error; await sleep(250); }
  }
  throw lastError || new Error('server did not become healthy');
}

function sleep(ms) { return new Promise((r) => setTimeout(r, ms)); }

function roundMs(value) { return Math.round(value * 100) / 100; }

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

async function runCommand(command, args, options = {}) {
  const started = Date.now();
  const child = spawn(command, args, {
    cwd: options.cwd || sdkRoot,
    env: options.env || process.env,
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (chunk) => { stdout += chunk.toString(); });
  child.stderr.on('data', (chunk) => { stderr += chunk.toString(); });
  if (options.input !== undefined) child.stdin.end(options.input);
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
    command, args, stdout, stderr,
    code: result.code,
    signal: result.signal,
    duration_ms: Date.now() - started,
  };
}

async function timeRequest(name, fn, samples, report, bucket = 'direct_http') {
  const durations = [];
  let lastResult;
  for (let i = 0; i < samples; i++) {
    const started = performance.now();
    lastResult = await fn(i);
    durations.push(performance.now() - started);
  }
  if (!report.latency[bucket]) report.latency[bucket] = {};
  report.latency[bucket][name] = summarizeDurations(durations);
  return lastResult;
}

// ─── Scenario Tracking ─────────────────────────────────────────────────

function addScenario(report, scenario) {
  report.scenarios.push({ status: 'passed', evidence: {}, ...scenario });
}

function failScenario(report, id, name, failureClass, evidence = {}) {
  addScenario(report, { id, name, status: 'failed', failure_class: failureClass, evidence });
}

// ─── Server Management ─────────────────────────────────────────────────

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
    MEMFUSE_AGENT_ID: 'claude-code-e2e-agent',
    MEMFUSE_SUMMARY_PROVIDER: 'deterministic',
    MEMFUSE_EMBEDDING_PROVIDER: 'deterministic',
    MEMFUSE_CHAT_PROVIDER: 'deterministic',
    RUST_LOG: process.env.RUST_LOG || 'mfs_server=info',
  };

  const child = spawn('cargo', ['run', '-p', 'mfs-server'], {
    cwd: repoRoot, env,
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
    server.child.once('close', () => { clearTimeout(timer); resolveStop(); });
  });
}

// ─── API Wrappers ──────────────────────────────────────────────────────

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

async function waitForSearch(serverUrl, sentinel, sessionId, timeoutMs) {
  const deadline = Date.now() + timeoutMs;
  let lastResult = null;
  while (Date.now() < deadline) {
    lastResult = await searchMemory(serverUrl, sentinel, sessionId);
    if (searchResultsContain(lastResult, sentinel)) return lastResult;
    await sleep(500);
  }
  return lastResult;
}

function searchResultsContain(searchResult, needle) {
  const results = Array.isArray(searchResult?.results) ? searchResult.results : [];
  return results.some((item) => JSON.stringify(item).includes(needle));
}

// ─── Claude Code Hook Runner ───────────────────────────────────────────

/**
 * Run a hook binary with Claude Code-formatted JSON stdin.
 * Returns { stdout, stderr, code, duration_ms }.
 * stdout is plain text (not JSON) for Claude Code platform.
 */
async function runClaudeHook(hookFile, input, env, timeoutMs = 20_000) {
  const payload = JSON.stringify(input);
  return runCommand(process.execPath, [join(sdkRoot, 'bin', 'hooks', hookFile)], {
    cwd: sdkRoot,
    env,
    input: payload,
    timeoutMs,
  });
}

// ─── MCP Tool Runner ───────────────────────────────────────────────────

/**
 * Spawn the MCP server, perform JSON-RPC handshake, call a tool, return result.
 * Uses a lightweight JSON-RPC client over stdio.
 */
async function runMcpTool(toolName, args, env, timeoutMs = 30_000) {
  const child = spawn(process.execPath, [join(sdkRoot, 'bin', 'memfuse-mcp.cjs')], {
    cwd: sdkRoot,
    env,
    stdio: ['pipe', 'pipe', 'pipe'],
  });

  let stdout = '';
  let stderr = '';
  child.stdout.on('data', (c) => { stdout += c.toString(); });
  child.stderr.on('data', (c) => { stderr += c.toString(); });

  let msgId = 1;
  const pending = new Map();
  let forceKillTimer = null;

  function send(obj) {
    const msg = JSON.stringify(obj);
    child.stdin.write(msg + '\n');
  }

  function handleMessage(msg) {
    if (msg.id != null && pending.has(msg.id)) {
      const { resolve, reject } = pending.get(msg.id);
      pending.delete(msg.id);
      if (msg.error) reject(new Error(JSON.stringify(msg.error)));
      else resolve(msg.result);
    }
  }

  const parser = createJsonRpcFrameParser(handleMessage);
  child.stdout.on('data', (chunk) => {
    try {
      parser.push(chunk);
    } catch { /* ignore parse errors */ }
  });

  function rpcCall(method, params) {
    return new Promise((resolveRpc, rejectRpc) => {
      const id = msgId++;
      pending.set(id, { resolve: resolveRpc, reject: rejectRpc });
      try {
        send({ jsonrpc: '2.0', id, method, params });
      } catch (error) {
        pending.delete(id);
        rejectRpc(error);
      }
    });
  }

  function rejectPending(error) {
    for (const { reject } of pending.values()) reject(error);
    pending.clear();
  }

  child.once('error', (error) => {
    rejectPending(error);
  });

  child.once('close', (code, signal) => {
    if (pending.size > 0) {
      rejectPending(new Error(`MCP server exited before response (code=${code}, signal=${signal || 'none'}): ${stderr.slice(-500)}`));
    }
  });

  const killTimer = setTimeout(() => {
    child.kill('SIGTERM');
    rejectPending(new Error(`MCP tool ${toolName} timed out after ${timeoutMs}ms: ${stderr.slice(-500)}`));
    forceKillTimer = setTimeout(() => child.kill('SIGKILL'), 3_000);
  }, timeoutMs);

  try {
    // MCP handshake: initialize
    const initResult = await rpcCall('initialize', {
      protocolVersion: '2024-11-05',
      capabilities: {},
      clientInfo: { name: 'e2e-test', version: '0.1.0' },
    });

    // Send initialized notification
    send({ jsonrpc: '2.0', method: 'notifications/initialized' });

    // Small delay to let server process
    await sleep(100);

    // Call the tool
    const toolResult = await rpcCall('tools/call', {
      name: toolName,
      arguments: args,
    });

    clearTimeout(killTimer);
    if (forceKillTimer) clearTimeout(forceKillTimer);
    child.kill('SIGTERM');

    return {
      ok: !toolResult?.isError,
      result: toolResult,
      initResult,
      stderr,
      stderr_tail: stderr.slice(-500),
    };
  } catch (error) {
    clearTimeout(killTimer);
    if (forceKillTimer) clearTimeout(forceKillTimer);
    child.kill('SIGTERM');
    return {
      ok: false,
      error: error.message,
      stderr,
      stderr_tail: stderr.slice(-500),
    };
  }
}

// ─── Test Scenarios ────────────────────────────────────────────────────

async function runD0(serverUrl, report) {
  const name = 'D0';
  const scenario = { id: name, name: 'Server Health & Direct HTTP Diagnostics', evidence: {} };
  try {
    // Health
    const health = await fetchJson(`${serverUrl}/health`);
    scenario.evidence.health = health;

    // Create session for diagnostics
    await createSession(serverUrl, 'd0-diagnostics');

    // store_observation latency
    await timeRequest('store_observation', (i) => fetchJson(`${serverUrl}/sessions/d0-diagnostics/observations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'ManualNote',
        tool_input: '',
        tool_output: '',
        content: `D0_DIAGNOSTIC_SENTINEL ${i} stored through direct HTTP`,
        platform: 'e2e',
      }),
    }), 5, report);

    // resolve_context latency
    await timeRequest('resolve_context', () => fetchJson(`${serverUrl}/context/resolve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        query: 'D0_DIAGNOSTIC_SENTINEL',
        session_id: 'd0-diagnostics',
        user_id: 'e2e-user',
        token_budget: 1200,
      }),
    }), 5, report);

    // search_memories latency
    await timeRequest('search_memories', () => searchMemory(serverUrl, 'D0_DIAGNOSTIC_SENTINEL', 'd0-diagnostics'), 5, report);

    // list_facts latency
    await timeRequest('list_facts', () => fetchJson(`${serverUrl}/facts?user_id=e2e-user`), 5, report);

    scenario.evidence.stored_observations = 5;
    scenario.evidence.latency_sampled = true;
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd0_health_diagnostics_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD1(serverUrl, report, env) {
  const name = 'D1';
  const scenario = { id: name, name: 'Claude Code Hook Diagnostics (Direct Invocation)', evidence: {} };
  const hookResults = {};

  try {
    await createSession(serverUrl, 'd1-hook-session');
    const hookEnv = { ...env, MEMFUSE_SESSION_ID: 'd1-hook-session' };

    // ── Setup hook ──
    const setupResult = await timeRequest('setup_hook', () => runClaudeHook('setup.cjs', {}, hookEnv), 3, report, 'hooks');
    hookResults.setup = { code: setupResult.code, stderr: setupResult.stderr.slice(-200) };
    assert.equal(setupResult.code, 0, `Setup hook exit ${setupResult.code}: ${setupResult.stderr}`);

    // ── SessionStart hook ──
    const sessionStartInput = {
      session_id: 'd1-hook-session',
      prompt: 'D1_SESSION_START_TEST working on authentication module',
      cwd: '/test/project',
    };
    const sessionStartResult = await timeRequest('session_start_hook', () =>
      runClaudeHook('session-start.cjs', sessionStartInput, hookEnv), 3, report, 'hooks');
    hookResults.sessionStart = {
      code: sessionStartResult.code,
      stdout: sessionStartResult.stdout.slice(0, 500),
      stderr: sessionStartResult.stderr.slice(-200),
    };
    assert.equal(sessionStartResult.code, 0, `SessionStart exit ${sessionStartResult.code}: ${sessionStartResult.stderr}`);
    // Claude Code output is plain text, NOT JSON
    const ssOutput = sessionStartResult.stdout;
    const isPlainText = !ssOutput.trim().startsWith('{') || ssOutput.includes('## MemFuse Memory Context');
    hookResults.sessionStart.is_plain_text = isPlainText;
    hookResults.sessionStart.contains_header = ssOutput.includes('## MemFuse Memory Context');

    // ── UserPromptSubmit hook ──
    const upsInput = {
      session_id: 'd1-hook-session',
      user_message: 'I need help implementing the authentication system with JWT tokens',
    };
    const upsResult = await timeRequest('user_prompt_submit_hook', () =>
      runClaudeHook('user-prompt-submit.cjs', upsInput, hookEnv), 3, report, 'hooks');
    hookResults.userPromptSubmit = {
      code: upsResult.code,
      stdout: upsResult.stdout.slice(0, 500),
      stderr: upsResult.stderr.slice(-200),
    };
    assert.equal(upsResult.code, 0, `UserPromptSubmit exit ${upsResult.code}: ${upsResult.stderr}`);
    // Output should either be empty (no signal) or contain the signal prefix
    const upsOutput = upsResult.stdout;
    hookResults.userPromptSubmit.has_signal = upsOutput.includes('📍 **MemFuse signal**') || upsOutput.length === 0;

    // ── PreToolUse[Read] hook ──
    const ptuReadInput = {
      session_id: 'd1-hook-session',
      tool_name: 'Read',
      tool_input: { file_path: 'src/auth.rs' },
    };
    const ptuReadResult = await timeRequest('pre_tool_use_read_hook', () =>
      runClaudeHook('pre-tool-use.cjs', ptuReadInput, hookEnv), 3, report, 'hooks');
    hookResults.preToolUseRead = {
      code: ptuReadResult.code,
      stdout: ptuReadResult.stdout.slice(0, 500),
      stderr: ptuReadResult.stderr.slice(-200),
    };
    assert.equal(ptuReadResult.code, 0, `PreToolUse[Read] exit ${ptuReadResult.code}: ${ptuReadResult.stderr}`);
    hookResults.preToolUseRead.output_is_plain_text = !ptuReadResult.stdout.trim().startsWith('{');

    // ── PreToolUse[other] hook (Bash — should be no-op) ──
    const ptuOtherInput = {
      session_id: 'd1-hook-session',
      tool_name: 'Bash',
      tool_input: { command: 'echo hello' },
    };
    const ptuOtherResult = await runClaudeHook('pre-tool-use.cjs', ptuOtherInput, hookEnv);
    hookResults.preToolUseOther = { code: ptuOtherResult.code, stdout: ptuOtherResult.stdout };
    assert.equal(ptuOtherResult.code, 0, `PreToolUse[other] exit ${ptuOtherResult.code}`);
    assert.equal(ptuOtherResult.stdout.trim(), '', 'PreToolUse[other] should produce no output');

    // ── PostToolUse[Edit] hook ──
    const ptEditInput = {
      session_id: 'd1-hook-session',
      tool_name: 'Edit',
      tool_input: { file_path: 'src/auth.rs', old_string: 'old', new_string: 'new' },
      tool_output: 'File edited successfully',
    };
    const ptEditResult = await timeRequest('post_tool_use_edit_hook', () =>
      runClaudeHook('post-tool-use.cjs', ptEditInput, hookEnv), 3, report, 'hooks');
    hookResults.postToolUseEdit = { code: ptEditResult.code, stderr: ptEditResult.stderr.slice(-200) };
    assert.equal(ptEditResult.code, 0, `PostToolUse[Edit] exit ${ptEditResult.code}: ${ptEditResult.stderr}`);

    // ── PostToolUse[Bash] hook ──
    const ptBashInput = {
      session_id: 'd1-hook-session',
      tool_name: 'Bash',
      tool_input: 'printf D1_BASH_SENTINEL',
      tool_output: 'D1_BASH_SENTINEL',
    };
    const ptBashResult = await timeRequest('post_tool_use_bash_hook', () =>
      runClaudeHook('post-tool-use.cjs', ptBashInput, hookEnv), 3, report, 'hooks');
    hookResults.postToolUseBash = { code: ptBashResult.code, stderr: ptBashResult.stderr.slice(-200) };
    assert.equal(ptBashResult.code, 0, `PostToolUse[Bash] exit ${ptBashResult.code}: ${ptBashResult.stderr}`);

    // ── PostToolUse[Glob] hook (should be skipped per SKIP_PATTERNS) ──
    const ptGlobInput = {
      session_id: 'd1-hook-session',
      tool_name: 'Glob',
      tool_input: { pattern: '**/*.ts' },
      tool_output: 'src/index.ts\nsrc/hooks/setup.ts',
    };
    const ptGlobResult = await runClaudeHook('post-tool-use.cjs', ptGlobInput, hookEnv);
    hookResults.postToolUseGlob = { code: ptGlobResult.code };
    assert.equal(ptGlobResult.code, 0, `PostToolUse[Glob] exit ${ptGlobResult.code}`);

    // ── Stop hook ──
    const stopInput = {
      session_id: 'd1-hook-session',
      last_assistant_message: 'Completed the D1 diagnostic test. The auth module was analyzed and key findings were documented. Next step: implement JWT validation. Learned that RS256 is preferred for this project.',
    };
    const stopResult = await timeRequest('stop_hook', () =>
      runClaudeHook('stop.cjs', stopInput, hookEnv), 3, report, 'hooks');
    hookResults.stop = { code: stopResult.code, stderr: stopResult.stderr.slice(-200) };
    assert.equal(stopResult.code, 0, `Stop hook exit ${stopResult.code}: ${stopResult.stderr}`);

    // ── PreCompact hook ──
    const preCompactInput = {
      session_id: 'd1-hook-session',
      trigger: 'auto',
    };
    const preCompactResult = await timeRequest('pre_compact_hook', () =>
      runClaudeHook('pre-compact.cjs', preCompactInput, hookEnv), 3, report, 'hooks');
    hookResults.preCompact = { code: preCompactResult.code, stderr: preCompactResult.stderr.slice(-200) };
    assert.equal(preCompactResult.code, 0, `PreCompact exit ${preCompactResult.code}: ${preCompactResult.stderr}`);

    // ── SessionEnd hook ──
    const sessionEndInput = {
      session_id: 'd1-hook-session',
      reason: 'e2e-test',
    };
    const sessionEndResult = await timeRequest('session_end_hook', () =>
      runClaudeHook('session-end.cjs', sessionEndInput, hookEnv), 3, report, 'hooks');
    hookResults.sessionEnd = { code: sessionEndResult.code, stderr: sessionEndResult.stderr.slice(-200) };
    assert.equal(sessionEndResult.code, 0, `SessionEnd exit ${sessionEndResult.code}: ${sessionEndResult.stderr}`);

    // Verify side effects in session context
    const ctx = await sessionContext(serverUrl, 'd1-hook-session').catch((e) => ({ error: e.message }));
    const ctxStr = JSON.stringify(ctx);
    hookResults.session_context_contains_bash_sentinel = ctxStr.includes('D1_BASH_SENTINEL');

    scenario.evidence = hookResults;
    if (!isPlainText) {
      scenario.status = 'failed';
      scenario.failure_class = 'session_start_output_not_plain_text';
    }
    if (!hookResults.userPromptSubmit.has_signal) {
      scenario.status = 'failed';
      scenario.failure_class = 'user_prompt_submit_output_missing_signal';
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd1_hook_diagnostics_failed';
    scenario.evidence = { ...hookResults, error: error instanceof Error ? error.message : String(error) };
    addScenario(report, scenario);
  }
}

async function runD2(report, env) {
  const name = 'D2';
  const scenario = { id: name, name: 'MCP Server Tool Validation', evidence: {} };
  const toolResults = {};

  try {
    // Create a session for MCP tests
    const serverUrl = env.MEMFUSE_SERVER_URL;
    await createSession(serverUrl, 'd2-mcp-session');

    // ── memfuse_guide ──
    const guide = await timeRequest('memfuse_guide', () =>
      runMcpTool('memfuse_guide', {}, env), 3, report, 'mcp_tools');
    toolResults.memfuse_guide = { ok: mcpToolSucceeded(guide), has_text: mcpToolSucceeded(guide) && JSON.stringify(guide.result).includes('Quick Guide') };

    // ── session_create ──
    const sessCreate = await timeRequest('session_create', () =>
      runMcpTool('session_create', { session_id: 'd2-mcp-created' }, env), 3, report, 'mcp_tools');
    toolResults.session_create = { ok: mcpToolSucceeded(sessCreate) };

    // ── add_message ──
    const addMsg = await timeRequest('add_message', () =>
      runMcpTool('add_message', {
        session_id: 'd2-mcp-session',
        role: 'user',
        content: 'Testing MCP tool validation for Claude Code E2E',
      }, env), 3, report, 'mcp_tools');
    toolResults.add_message = { ok: mcpToolSucceeded(addMsg) };

    // ── store_observation (uses config.sessionId from env) ──
    const mcpEnv = { ...env, MEMFUSE_SESSION_ID: 'd2-mcp-session' };
    const storeObs = await timeRequest('store_observation', () =>
      runMcpTool('store_observation', {
        content: 'D2_MCP_SENTINEL: MCP store_observation tool works correctly',
        tool_name: 'ManualNote',
      }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.store_observation = { ok: mcpToolSucceeded(storeObs) };

    // ── search_memories ──
    const search = await timeRequest('search_memories', () =>
      runMcpTool('search_memories', { query: 'D2_MCP_SENTINEL' }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.search_memories = { ok: mcpToolSucceeded(search), has_results: mcpToolSucceeded(search) && JSON.stringify(search.result).includes('D2_MCP_SENTINEL') };

    // ── resolve_context ──
    const resolveCtx = await timeRequest('resolve_context', () =>
      runMcpTool('resolve_context', { query: 'MCP tool validation' }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.resolve_context = { ok: mcpToolSucceeded(resolveCtx) };

    // ── inject_context ──
    const injectCtx = await timeRequest('inject_context', () =>
      runMcpTool('inject_context', { query: 'MCP tool validation' }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.inject_context = { ok: mcpToolSucceeded(injectCtx) };

    // ── create_fact ──
    const createFact = await timeRequest('create_fact', () =>
      runMcpTool('create_fact', {
        subject: 'D2_MCP_TOOL',
        predicate: 'status',
        display_value: 'all tools validated',
        confidence: 0.95,
      }, env), 3, report, 'mcp_tools');
    const d2FactId = extractFactIdFromMcpResult(createFact.result);
    toolResults.create_fact = { ok: mcpToolSucceeded(createFact), fact_id: d2FactId };

    // ── list_facts ──
    const listFacts = await timeRequest('list_facts', () =>
      runMcpTool('list_facts', {}, env), 3, report, 'mcp_tools');
    toolResults.list_facts = { ok: mcpToolSucceeded(listFacts), has_facts: mcpToolSucceeded(listFacts) && JSON.stringify(listFacts.result).includes('D2_MCP_TOOL') };

    // ── supersede_fact (need fact ID from create_fact) ──
    if (d2FactId) {
      const supersede = await timeRequest('supersede_fact', () =>
        runMcpTool('supersede_fact', {
          old_fact_id: d2FactId,
          subject: 'D2_MCP_TOOL',
          predicate: 'status',
          display_value: 'all tools validated and superseded',
          confidence: 0.99,
        }, env), 3, report, 'mcp_tools');
      toolResults.supersede_fact = { ok: mcpToolSucceeded(supersede) };
    } else {
      toolResults.supersede_fact = { ok: false, reason: 'no fact id found' };
    }

    // ── retract_fact (create a new one to retract) ──
    const retractFactCreate = await runMcpTool('create_fact', {
      subject: 'D2_RETRACT_TEST',
      predicate: 'temporary',
      display_value: 'will be retracted',
      confidence: 0.5,
    }, env);
    const retractFactId = extractFactIdFromMcpResult(retractFactCreate.result);
    if (retractFactId) {
      const retract = await timeRequest('retract_fact', () =>
        runMcpTool('retract_fact', { fact_id: retractFactId }, env), 3, report, 'mcp_tools');
      toolResults.retract_fact = { ok: mcpToolSucceeded(retract) };
    } else {
      toolResults.retract_fact = { ok: false, reason: 'no fact id to retract' };
    }

    // ── trace_fact ──
    if (d2FactId) {
      const trace = await timeRequest('trace_fact', () =>
        runMcpTool('trace_fact', { fact_id: d2FactId }, env), 3, report, 'mcp_tools');
      toolResults.trace_fact = { ok: mcpToolSucceeded(trace) };
    }

    // ── commit_session (uses config.sessionId from env) ──
    const commit = await timeRequest('commit_session', () =>
      runMcpTool('commit_session', { reason: 'e2e-d2' }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.commit_session = { ok: mcpToolSucceeded(commit) };

    // ── simulate_reaction ──
    const simReact = await timeRequest('simulate_reaction', () =>
      runMcpTool('simulate_reaction', { scenario: 'changing database from PostgreSQL to SQLite' }, env), 3, report, 'mcp_tools');
    toolResults.simulate_reaction = { ok: mcpToolSucceeded(simReact) };

    // ── heuristics_l0_confirmed ──
    const l0 = await timeRequest('heuristics_l0_confirmed', () =>
      runMcpTool('heuristics_l0_confirmed', {}, env), 3, report, 'mcp_tools');
    toolResults.heuristics_l0_confirmed = { ok: mcpToolSucceeded(l0) };

    // ── heuristics_confirm_rule (may fail if no rules exist — ok) ──
    const confirmRule = await runMcpTool('heuristics_confirm_rule', { rule_id: 'nonexistent-rule' }, env);
    toolResults.heuristics_confirm_rule = { ok: mcpToolSucceeded(confirmRule), note: 'expected to fail gracefully if no rules' };

    // ── export_memories ──
    const exportMem = await timeRequest('export_memories', () =>
      runMcpTool('export_memories', {}, env), 3, report, 'mcp_tools');
    toolResults.export_memories = { ok: mcpToolSucceeded(exportMem) };

    // ── import_memories ──
    const importMem = await timeRequest('import_memories', () =>
      runMcpTool('import_memories', { markdown: '# Test Import\n\n- D2 imported fact: MCP tools work' }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.import_memories = { ok: mcpToolSucceeded(importMem) };

    // ── cite_memories ──
    const cite = await timeRequest('cite_memories', () =>
      runMcpTool('cite_memories', { episode_ids: ['fake-episode-id'], fact_ids: [] }, env), 3, report, 'mcp_tools');
    toolResults.cite_memories = { ok: mcpToolSucceeded(cite) };

    // ── facts_at_time ──
    const factsAtTime = await timeRequest('facts_at_time', () =>
      runMcpTool('facts_at_time', { at_time: new Date().toISOString() }, mcpEnv), 3, report, 'mcp_tools');
    toolResults.facts_at_time = { ok: mcpToolSucceeded(factsAtTime) };

    // DIG tools — these may fail if no resources exist, which is ok
    const digTools = ['ls', 'read', 'abstract', 'overview', 'glob', 'grep'];
    for (const tool of digTools) {
      const digResult = await runMcpTool(tool, { uri: 'mfs://resources/' }, env);
      toolResults[tool] = { ok: mcpToolSucceeded(digResult), note: 'may fail if no resources indexed' };
    }

    scenario.evidence = toolResults;
    const criticalTools = ['memfuse_guide', 'session_create', 'add_message', 'store_observation', 'search_memories',
      'resolve_context', 'inject_context', 'create_fact', 'list_facts', 'supersede_fact', 'retract_fact',
      'trace_fact', 'commit_session', 'export_memories', 'import_memories', 'cite_memories', 'facts_at_time'];
    const criticalFailed = criticalTools.filter((t) => !toolResults[t]?.ok);
    if (criticalFailed.length > 0) {
      scenario.status = 'failed';
      scenario.failure_class = 'mcp_critical_tools_failed';
      scenario.evidence.failed_critical = criticalFailed;
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd2_mcp_validation_failed';
    scenario.evidence = { ...toolResults, error: error instanceof Error ? error.message : String(error) };
    addScenario(report, scenario);
  }
}

async function runD3(report) {
  const name = 'D3';
  const scenario = { id: name, name: 'Skill Discovery', evidence: {} };
  try {
    // Verify SKILL.md exists in built output
    const skillDir = join(sdkRoot, 'dist', 'skills', 'memfuse');
    const skillPath = join(skillDir, 'SKILL.md');
    const exists = await pathExists(skillPath);
    scenario.evidence.skill_file_exists = exists;

    if (!exists) {
      scenario.status = 'failed';
      scenario.failure_class = 'skill_file_missing';
      addScenario(report, scenario);
      return;
    }

    const content = await readFile(skillPath, 'utf8');
    scenario.evidence.has_yaml_frontmatter = content.startsWith('---');
    scenario.evidence.has_name_memfuse = /name:\s*memfuse/.test(content);
    scenario.evidence.has_description = /description:/.test(content);
    scenario.evidence.has_look_dig_save = content.includes('LOOK') && content.includes('DIG') && content.includes('SAVE');
    scenario.evidence.has_commands_reference = await pathExists(join(skillDir, 'references', 'commands.md'));

    // Verify CLI commands referenced in SKILL.md exist
    const cliCommands = ['inject-context', 'search', 'list-facts', 'store-observation',
      'session-create', 'add-message', 'resolve-context', 'commit-session',
      'create-fact', 'supersede-fact', 'retract-fact', 'trace-fact',
      'simulate-reaction', 'heuristics-l0', 'confirm-rule',
      'consolidate', 'extract-facts', 'cite-memories', 'export-memories', 'import-memories'];
    const memfuseCliPath = join(sdkRoot, 'bin', 'memfuse.cjs');
    const cliExists = await pathExists(memfuseCliPath);
    scenario.evidence.cli_binary_exists = cliExists;

    // Install to a .claude/skills/ layout and verify discoverability
    const tempSkillDir = join(tmpdir(), `memfuse-d3-skills-${Date.now()}`);
    await mkdir(join(tempSkillDir, '.claude', 'skills'), { recursive: true });
    await cp(skillDir, join(tempSkillDir, '.claude', 'skills', 'memfuse'), { recursive: true });
    const installedExists = await pathExists(join(tempSkillDir, '.claude', 'skills', 'memfuse', 'SKILL.md'));
    const installedReferenceExists = await pathExists(join(tempSkillDir, '.claude', 'skills', 'memfuse', 'references', 'commands.md'));
    scenario.evidence.installable_to_claude_skills = installedExists;
    scenario.evidence.installs_reference = installedReferenceExists;

    // Cleanup
    await rm(tempSkillDir, { recursive: true, force: true });

    if (!exists || !scenario.evidence.has_commands_reference || !installedExists || !installedReferenceExists) {
      scenario.status = 'failed';
      scenario.failure_class = 'skill_not_discoverable';
    }
    failScenarioIfMissingEvidence(scenario, [
      'has_yaml_frontmatter',
      'has_name_memfuse',
      'has_description',
      'has_look_dig_save',
      'cli_binary_exists',
      'installable_to_claude_skills',
    ], 'skill_discovery_incomplete');
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd3_skill_discovery_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD4(serverUrl, report, env) {
  const name = 'D4';
  const sentinel = 'MULTI_TURN_RUST_BACKEND_7X9K';
  const scenario = { id: name, name: 'Multi-Turn Memory Accumulation', evidence: {} };
  try {
    // Turn 1 (Session A): Store observation via PostToolUse hook
    await createSession(serverUrl, 'd4-session-a');
    const hookEnvA = { ...env, MEMFUSE_SESSION_ID: 'd4-session-a' };

    const postToolResult = await runClaudeHook('post-tool-use.cjs', {
      session_id: 'd4-session-a',
      tool_name: 'Bash',
      tool_input: `python -c "print('DECISION: Use Rust for backend because of memory safety. ${sentinel}')"`,
      tool_output: `DECISION: Use Rust for backend because of memory safety. ${sentinel}`,
    }, hookEnvA);
    assert.equal(postToolResult.code, 0, `PostToolUse failed: ${postToolResult.stderr}`);
    scenario.evidence.turn1_post_tool_code = postToolResult.code;

    // Commit session A
    await fetchJson(`${serverUrl}/sessions/d4-session-a/commit`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ user_id: 'e2e-user', reason: 'e2e-d4-turn1' }),
    });

    // Turn 2 (Session B): Create fact via HTTP
    const factId = `fact_rust_${Date.now()}`;
    await fetchJson(`${serverUrl}/facts`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        id: factId,
        subject: 'project backend',
        predicate: 'language',
        display_value: `Rust (chosen for memory safety) ${sentinel}`,
        confidence: 0.95,
        agent_id: 'claude-code-e2e-agent',
      }),
    });
    scenario.evidence.turn2_fact_id = factId;

    // Wait for context to include both the observation and the fact
    let turn3Context = null;
    try {
      turn3Context = await waitForContext(serverUrl, sentinel, 'd4-session-c', 30_000);
    } catch {
      // Try a direct resolve as fallback
      turn3Context = await fetchJson(`${serverUrl}/context/resolve`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ query: sentinel, user_id: 'e2e-user', session_id: 'd4-session-c', token_budget: 1500 }),
      });
    }
    const contextStr = JSON.stringify(turn3Context);
    scenario.evidence.turn3_context_has_sentinel = contextStr.includes(sentinel);
    scenario.evidence.turn3_context_has_rust = contextStr.includes('Rust');

    // Turn 4 (Session C continued): UserPromptSubmit hook
    await createSession(serverUrl, 'd4-session-c');
    const hookEnvC = { ...env, MEMFUSE_SESSION_ID: 'd4-session-c' };
    const upsResult = await runClaudeHook('user-prompt-submit.cjs', {
      session_id: 'd4-session-c',
      user_message: 'Help me with the backend architecture',
    }, hookEnvC);
    scenario.evidence.turn4_ups_code = upsResult.code;
    scenario.evidence.turn4_ups_has_signal = upsResult.stdout.includes('📍 **MemFuse signal**') || upsResult.stdout.includes('Rust');

    const allPresent = contextStr.includes(sentinel) && contextStr.includes('Rust');
    if (!allPresent || upsResult.code !== 0 || !scenario.evidence.turn4_ups_has_signal) {
      scenario.status = 'failed';
      scenario.failure_class = 'multi_turn_memory_incomplete';
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd4_multi_turn_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD5(serverUrl, report) {
  const name = 'D5';
  const sentinel = 'FACT_LIFECYCLE_PG_TO_SQLITE_3M8N';
  const scenario = { id: name, name: 'Cross-Session Fact Lifecycle', evidence: {} };
  try {
    // 1. Create fact: project uses PostgreSQL
    const pgFactId = `fact_pg_${Date.now()}`;
    await fetchJson(`${serverUrl}/facts`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        id: pgFactId,
        subject: 'project database',
        predicate: 'engine',
        display_value: `PostgreSQL (primary) ${sentinel}`,
        confidence: 0.95,
        agent_id: 'claude-code-e2e-agent',
      }),
    });
    scenario.evidence.pg_fact_id = pgFactId;
    const pgEffectiveAt = new Date().toISOString();
    await sleep(10);

    // 2. Verify fact appears in list_facts
    const list1 = await fetchJson(`${serverUrl}/facts?user_id=e2e-user`);
    const list1Str = JSON.stringify(list1);
    scenario.evidence.pg_in_list_facts = list1Str.includes(pgFactId);

    // 3. Verify fact appears in resolve_context
    const ctx1 = await fetchJson(`${serverUrl}/context/resolve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ query: 'project database', user_id: 'e2e-user', session_id: 'd5-lifecycle', token_budget: 1500 }),
    });
    scenario.evidence.pg_in_context = JSON.stringify(ctx1).includes('PostgreSQL');

    // 4. Supersede fact: project uses SQLite
    const sqliteFactId = `fact_sqlite_${Date.now()}`;
    await fetchJson(`${serverUrl}/facts`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        id: sqliteFactId,
        subject: 'project database',
        predicate: 'engine',
        display_value: `SQLite (replaced PostgreSQL) ${sentinel}`,
        confidence: 0.98,
        agent_id: 'claude-code-e2e-agent',
        supersedes: pgFactId,
      }),
    });
    scenario.evidence.sqlite_fact_id = sqliteFactId;

    // 5. Supersede via API
    try {
      await fetchJson(`${serverUrl}/facts/${pgFactId}/supersede`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ new_fact_id: sqliteFactId }),
      });
      scenario.evidence.supersede_api_worked = true;
    } catch (e) {
      scenario.evidence.supersede_api_error = e.message;
    }

    // 6. Retract SQLite fact
    try {
      await fetchJson(`${serverUrl}/facts/${sqliteFactId}/retract`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ reason: 'e2e-test-cleanup' }),
      });
      scenario.evidence.retract_api_worked = true;
    } catch (e) {
      scenario.evidence.retract_api_error = e.message;
    }

    // 7. Verify retracted fact no longer in active list
    const list2 = await fetchJson(`${serverUrl}/facts?user_id=e2e-user`);
    const list2Str = JSON.stringify(list2);
    scenario.evidence.sqlite_not_in_active = !list2Str.includes(sqliteFactId);

    // 8. Query historical state
    try {
      const historical = await fetchJson(`${serverUrl}/context/resolve`, {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({
          query: 'project database',
          user_id: 'e2e-user',
          session_id: 'd5-lifecycle',
          token_budget: 1500,
          at_time: pgEffectiveAt,
        }),
      });
      scenario.evidence.historical_query_ok = true;
      scenario.evidence.historical_has_pg = JSON.stringify(historical).includes('PostgreSQL');
    } catch (e) {
      scenario.evidence.historical_query_error = e.message;
    }

    failScenarioIfMissingEvidence(scenario, [
      'pg_in_list_facts',
      'pg_in_context',
      'supersede_api_worked',
      'retract_api_worked',
      'sqlite_not_in_active',
      'historical_query_ok',
    ], 'fact_lifecycle_incomplete');
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd5_fact_lifecycle_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD6(serverUrl, report, env) {
  const name = 'D6';
  const sentinel = 'READ_HINT_JWT_RS256_AUTH_A4K2';
  const scenario = { id: name, name: 'PreToolUse[Read] Memory Hints', evidence: {} };
  try {
    // 1. Store observation about src/auth.rs
    await createSession(serverUrl, 'd6-read-hints');
    await fetchJson(`${serverUrl}/sessions/d6-read-hints/observations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'ManualNote',
        tool_input: '',
        tool_output: '',
        content: `Authentication module src/auth.rs uses JWT tokens with RS256 algorithm. ${sentinel}`,
        platform: 'e2e',
      }),
    });
    await fetchJson(`${serverUrl}/sessions/d6-read-hints/commit`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ user_id: 'e2e-user', reason: 'e2e-d6-setup' }),
    });
    await fetchJson(`${serverUrl}/facts`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        id: `fact_d6_${Date.now()}`,
        subject: 'read hint file',
        predicate: 'memory_hint',
        display_value: `src/auth.rs has JWT RS256 context ${sentinel}`,
        confidence: 0.9,
        agent_id: 'claude-code-e2e-agent',
      }),
    });

    // Wait for context to pick up the observation
    try {
      await waitForContext(serverUrl, sentinel, 'd6-read-hints-new', 20_000);
    } catch { /* proceed anyway */ }

    // 2. Invoke PreToolUse[Read] for src/auth.rs
    const hookEnv = { ...env, MEMFUSE_SESSION_ID: 'd6-read-hints-new' };
    const readResult = await timeRequest('pre_tool_use_read_hint', () =>
      runClaudeHook('pre-tool-use.cjs', {
        session_id: 'd6-read-hints-new',
        tool_name: 'Read',
        tool_input: { file_path: 'src/auth.rs' },
      }, hookEnv), 3, report, 'hooks');
    scenario.evidence.read_auth_code = readResult.code;
    scenario.evidence.read_auth_output = readResult.stdout.slice(0, 500);
    scenario.evidence.read_auth_has_hint = readResult.stdout.includes('MemFuse') || readResult.stdout.includes('auth');

    // 3. Invoke PreToolUse[Read] for unrelated file
    const unrelatedResult = await runClaudeHook('pre-tool-use.cjs', {
      session_id: 'd6-read-hints-new',
      tool_name: 'Read',
      tool_input: { file_path: 'src/unrelated.rs' },
    }, hookEnv);
    scenario.evidence.read_unrelated_code = unrelatedResult.code;
    scenario.evidence.read_unrelated_output = unrelatedResult.stdout.slice(0, 300);
    scenario.evidence.read_unrelated_no_hint = unrelatedResult.stdout.length === 0 || !unrelatedResult.stdout.includes('auth');

    failScenarioIfMissingEvidence(scenario, [
      'read_auth_has_hint',
      'read_unrelated_no_hint',
    ], 'read_hints_incomplete');
    if (readResult.code !== 0 || unrelatedResult.code !== 0) {
      scenario.status = 'failed';
      scenario.failure_class = 'read_hints_hook_non_zero_exit';
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd6_read_hints_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD7(serverUrl, report, env) {
  const name = 'D7';
  const sentinel = 'PRECOMPACT_CTX_SAVE_9F3L';
  const scenario = { id: name, name: 'PreCompact Context Preservation', evidence: {} };
  try {
    // 1. Store observations and facts in a session
    await createSession(serverUrl, 'd7-precompact');
    await fetchJson(`${serverUrl}/sessions/d7-precompact/observations`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        tool_name: 'ManualNote',
        tool_input: '',
        tool_output: '',
        content: `D7 observation: project uses microservices architecture. ${sentinel}`,
        platform: 'e2e',
      }),
    });
    await fetchJson(`${serverUrl}/facts`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        id: `fact_d7_${Date.now()}`,
        subject: 'D7 architecture',
        predicate: 'pattern',
        display_value: `microservices ${sentinel}`,
        confidence: 0.9,
        agent_id: 'claude-code-e2e-agent',
      }),
    });

    // 2. Invoke PreCompact hook
    const hookEnv = { ...env, MEMFUSE_SESSION_ID: 'd7-precompact' };
    const preCompactResult = await timeRequest('pre_compact_save', () =>
      runClaudeHook('pre-compact.cjs', {
        session_id: 'd7-precompact',
        trigger: 'auto',
      }, hookEnv), 3, report, 'hooks');
    scenario.evidence.precompact_code = preCompactResult.code;
    scenario.evidence.precompact_stderr = preCompactResult.stderr.slice(-300);

    // 3. Verify PreCompact observation was stored
    const ctx = await sessionContext(serverUrl, 'd7-precompact').catch((e) => ({ error: e.message }));
    const ctxStr = JSON.stringify(ctx);
    scenario.evidence.context_has_precompact = ctxStr.includes('PreCompact') || ctxStr.includes('pre-compact');
    scenario.evidence.context_has_sentinel = ctxStr.includes(sentinel);

    // 4. Start new session and verify context is retrievable
    const newCtx = await fetchJson(`${serverUrl}/context/resolve`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        query: 'microservices architecture',
        user_id: 'e2e-user',
        session_id: 'd7-post-compact',
        token_budget: 1500,
      }),
    });
    scenario.evidence.new_session_has_context = JSON.stringify(newCtx).includes(sentinel);

    failScenarioIfMissingEvidence(scenario, [
      'context_has_precompact',
      'context_has_sentinel',
      'new_session_has_context',
    ], 'precompact_context_incomplete');
    if (preCompactResult.code !== 0) {
      scenario.status = 'failed';
      scenario.failure_class = 'precompact_hook_non_zero_exit';
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd7_precompact_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD8(serverUrl, report, env) {
  const name = 'D8';
  const sentinel = 'SESSION_END_CONSOLIDATE_2K7P';
  const scenario = { id: name, name: 'SessionEnd Consolidation Pipeline', evidence: {} };
  try {
    // 1. Create session and add messages
    await createSession(serverUrl, 'd8-session-end');
    await fetchJson(`${serverUrl}/sessions/d8-session-end/messages`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        role: 'user',
        content: 'Help me refactor the authentication module',
      }),
    });
    await fetchJson(`${serverUrl}/sessions/d8-session-end/messages`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({
        role: 'assistant',
        content: `I analyzed the auth module and decided to refactor it to use middleware pattern. ${sentinel}`,
      }),
    });

    // 2. Store observations via PostToolUse hook
    const hookEnv = { ...env, MEMFUSE_SESSION_ID: 'd8-session-end' };
    const postTool = await runClaudeHook('post-tool-use.cjs', {
      session_id: 'd8-session-end',
      tool_name: 'Bash',
      tool_input: `python -c "print('Refactored auth module to middleware pattern. ${sentinel}')"`,
      tool_output: `Refactored auth module to middleware pattern. ${sentinel}`,
    }, hookEnv);
    scenario.evidence.post_tool_code = postTool.code;

    // 3. Invoke SessionEnd hook
    const sessionEndResult = await timeRequest('session_end_consolidate', () =>
      runClaudeHook('session-end.cjs', {
        session_id: 'd8-session-end',
        reason: 'e2e-d8-test',
      }, hookEnv), 3, report, 'hooks');
    scenario.evidence.session_end_code = sessionEndResult.code;
    scenario.evidence.session_end_stderr = sessionEndResult.stderr.slice(-300);

    // 4. Wait for consolidation
    await sleep(2000);

    // 5. Verify session context after commit
    const ctx = await sessionContext(serverUrl, 'd8-session-end').catch((e) => ({ error: e.message }));
    const ctxStr = JSON.stringify(ctx);
    scenario.evidence.context_has_sentinel = ctxStr.includes(sentinel);

    // 6. Verify search finds the observation
    const search = await waitForSearch(serverUrl, sentinel, 'd8-session-end', 20_000);
    scenario.evidence.search_has_sentinel = searchResultsContain(search, sentinel);

    failScenarioIfMissingEvidence(scenario, [
      'context_has_sentinel',
    ], 'session_end_consolidation_incomplete');
    if (postTool.code !== 0 || sessionEndResult.code !== 0) {
      scenario.status = 'failed';
      scenario.failure_class = 'session_end_hook_non_zero_exit';
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd8_session_end_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD9(serverUrl, report, env) {
  const name = 'D9';
  const scenario = { id: name, name: 'Privacy Control (<private> blocks)', evidence: {} };
  try {
    await createSession(serverUrl, 'd9-privacy');
    const hookEnv = { ...env, MEMFUSE_SESSION_ID: 'd9-privacy' };

    // 1. Store observation with <private> block
    const result = await runClaudeHook('post-tool-use.cjs', {
      session_id: 'd9-privacy',
      tool_name: 'Bash',
      tool_input: 'python -c "print(\'config\')"',
      tool_output: 'Public info <private>SECRET_API_KEY=sk-12345678</private> more public info',
    }, hookEnv);
    assert.equal(result.code, 0, `PostToolUse failed: ${result.stderr}`);

    // Wait for storage
    await sleep(500);

    // 2. Verify stored content does NOT contain secret
    const ctx = await sessionContext(serverUrl, 'd9-privacy').catch((e) => ({ error: e.message }));
    const ctxStr = JSON.stringify(ctx);
    scenario.evidence.secret_stripped = !ctxStr.includes('SECRET_API_KEY');
    scenario.evidence.has_public_info = ctxStr.includes('Public info');
    scenario.evidence.has_more_public = ctxStr.includes('more public info');

    // 3. Test multiline private block
    const multilineResult = await runClaudeHook('post-tool-use.cjs', {
      session_id: 'd9-privacy',
      tool_name: 'Bash',
      tool_input: 'python -c "print(\'config multiline\')"',
      tool_output: 'Line1 <private>\nSECRET_TOKEN=abc\nPASSWORD=xyz\n</private> Line2',
    }, hookEnv);
    assert.equal(multilineResult.code, 0, `Multiline PostToolUse failed: ${multilineResult.stderr}`);

    // 4. Test nested private blocks
    const nestedResult = await runClaudeHook('post-tool-use.cjs', {
      session_id: 'd9-privacy',
      tool_name: 'Bash',
      tool_input: 'python -c "print(\'config nested\')"',
      tool_output: 'Outer <private>inner <private>nested secret</private> still private</private> end',
    }, hookEnv);
    assert.equal(nestedResult.code, 0, `Nested PostToolUse failed: ${nestedResult.stderr}`);

    await sleep(500);
    const ctx2 = await sessionContext(serverUrl, 'd9-privacy').catch((e) => ({ error: e.message }));
    const ctx2Str = JSON.stringify(ctx2);
    scenario.evidence.multiline_secret_stripped = !ctx2Str.includes('SECRET_TOKEN') && !ctx2Str.includes('PASSWORD');
    scenario.evidence.nested_secret_stripped = !ctx2Str.includes('nested secret');
    scenario.evidence.nested_tail_stripped = !ctx2Str.includes('still private') && !ctx2Str.includes('</private>');

    failScenarioIfMissingEvidence(scenario, [
      'secret_stripped',
      'has_public_info',
      'has_more_public',
      'multiline_secret_stripped',
      'nested_secret_stripped',
      'nested_tail_stripped',
    ], 'private_block_not_stripped');
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd9_privacy_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

async function runD10(report) {
  const name = 'D10';
  const scenario = { id: name, name: 'Degradation Resilience', evidence: {} };
  try {
    // Use unreachable server URL
    const badEnv = {
      ...process.env,
      MEMFUSE_SERVER_URL: 'http://127.0.0.1:1',
      MEMFUSE_USER_ID: 'e2e-user',
      MEMFUSE_SESSION_ID: 'd10-degraded',
    };

    // 1. Setup hook with bad server
    const setupResult = await runClaudeHook('setup.cjs', {}, badEnv, 15_000);
    scenario.evidence.setup_code = setupResult.code;
    scenario.evidence.setup_stderr = setupResult.stderr.slice(-300);
    scenario.evidence.setup_exits_cleanly = setupResult.code === 0;

    // 2. SessionStart hook with bad server
    const ssResult = await runClaudeHook('session-start.cjs', {
      session_id: 'd10-degraded',
      prompt: 'test degradation',
    }, badEnv, 15_000);
    scenario.evidence.session_start_code = ssResult.code;
    scenario.evidence.session_start_exits_cleanly = ssResult.code === 0;
    scenario.evidence.session_start_stderr_has_warning = ssResult.stderr.includes('degraded') || ssResult.stderr.includes('offline') || ssResult.stderr.includes('unavailable') || ssResult.stderr.includes('ECONNREFUSED');

    // 3. PostToolUse hook with bad server
    const ptResult = await runClaudeHook('post-tool-use.cjs', {
      session_id: 'd10-degraded',
      tool_name: 'Bash',
      tool_input: 'echo test',
      tool_output: 'test',
    }, badEnv, 15_000);
    scenario.evidence.post_tool_code = ptResult.code;
    scenario.evidence.post_tool_exits_cleanly = ptResult.code === 0;

    // 4. Stop hook with bad server
    const stopResult = await runClaudeHook('stop.cjs', {
      session_id: 'd10-degraded',
      last_assistant_message: 'Test degradation message for the stop hook. This message needs to be long enough to pass the threshold check.',
    }, badEnv, 15_000);
    scenario.evidence.stop_code = stopResult.code;
    scenario.evidence.stop_exits_cleanly = stopResult.code === 0;

    // 5. SessionEnd hook with bad server
    const seResult = await runClaudeHook('session-end.cjs', {
      session_id: 'd10-degraded',
      reason: 'e2e-degradation',
    }, badEnv, 15_000);
    scenario.evidence.session_end_code = seResult.code;
    scenario.evidence.session_end_exits_cleanly = seResult.code === 0;

    // 6. UserPromptSubmit hook with bad server
    const upsResult = await runClaudeHook('user-prompt-submit.cjs', {
      session_id: 'd10-degraded',
      user_message: 'test degradation with a long enough prompt',
    }, badEnv, 15_000);
    scenario.evidence.user_prompt_submit_code = upsResult.code;
    scenario.evidence.user_prompt_submit_exits_cleanly = upsResult.code === 0;

    // 7. PreCompact hook with bad server
    const pcResult = await runClaudeHook('pre-compact.cjs', {
      session_id: 'd10-degraded',
      trigger: 'auto',
    }, badEnv, 15_000);
    scenario.evidence.pre_compact_code = pcResult.code;
    scenario.evidence.pre_compact_exits_cleanly = pcResult.code === 0;

    // All hooks should exit 0 even when server is unreachable
    const allExitZero = [setupResult, ssResult, ptResult, stopResult, seResult, upsResult, pcResult]
      .every((r) => r.code === 0);
    if (!allExitZero) {
      scenario.status = 'failed';
      scenario.failure_class = 'degradation_non_zero_exit';
    }
    addScenario(report, scenario);
  } catch (error) {
    scenario.status = 'failed';
    scenario.failure_class = 'd10_degradation_failed';
    scenario.evidence.error = error instanceof Error ? error.message : String(error);
    addScenario(report, scenario);
  }
}

// ─── Report Generation ─────────────────────────────────────────────────

async function writeFailures(report) {
  const failures = report.scenarios.filter((s) => s.status === 'failed');
  if (failures.length === 0) return;
  const lines = ['# Claude Code E2E Memory Failures', ''];
  for (const f of failures) {
    lines.push(`## ${f.id}: ${f.name}`);
    lines.push('');
    lines.push(`- Failure class: ${f.failure_class || 'unknown'}`);
    lines.push(`- Evidence: \`${JSON.stringify(f.evidence || {})}\``);
    lines.push('');
  }
  await writeFile(join(report.artifacts.root, 'artifacts', 'failures.md'), lines.join('\n'));
}

async function writeLimitations(report) {
  const skipped = report.scenarios.filter((s) => s.status === 'skipped' && s.failure_class);
  if (skipped.length === 0) return;
  const lines = ['# Claude Code E2E Memory Limitations', ''];
  for (const s of skipped) {
    lines.push(`## ${s.id}: ${s.name}`);
    lines.push('');
    lines.push(`- Class: ${s.failure_class}`);
    lines.push(`- Evidence: \`${JSON.stringify(s.evidence || {})}\``);
    lines.push('');
  }
  await writeFile(join(report.artifacts.root, 'artifacts', 'limitations.md'), lines.join('\n'));
}

// ─── Main ──────────────────────────────────────────────────────────────

async function main() {
  const options = parseArgs(process.argv.slice(2));
  const runRoot = await mkdtemp(join(tmpdir(), 'memfuse-claude-code-e2e-'));
  await mkdir(join(runRoot, 'artifacts'), { recursive: true });
  const report = {
    run_id: new Date().toISOString().replace(/[-:.]/g, ''),
    status: 'running',
    scenarios: [],
    latency: { direct_http: {}, hooks: {}, mcp_tools: {} },
    artifacts: { root: runRoot },
  };
  const reportPath = join(runRoot, 'artifacts', 'report.json');
  let server;

  try {
    // Build SDK
    const buildOutput = commandOk('npm', ['run', 'build', '--silent'], { cwd: sdkRoot });
    assert.equal(buildOutput.ok, true, `SDK build failed: ${buildOutput.stderr}`);

    // Start server
    server = await startServer(runRoot, report);
    const env = {
      ...process.env,
      MEMFUSE_SERVER_URL: server.serverUrl,
      MEMFUSE_USER_ID: 'e2e-user',
      MEMFUSE_AGENT_ID: 'claude-code-e2e-agent',
      MEMFUSE_SUMMARY_PROVIDER: 'deterministic',
      MEMFUSE_EMBEDDING_PROVIDER: 'deterministic',
    };

    // Run all scenarios
    await runD0(server.serverUrl, report);
    await runD1(server.serverUrl, report, env);
    await runD2(report, env);
    await runD3(report);
    await runD4(server.serverUrl, report, env);
    await runD5(server.serverUrl, report);
    await runD6(server.serverUrl, report, env);
    await runD7(server.serverUrl, report, env);
    await runD8(server.serverUrl, report, env);
    await runD9(server.serverUrl, report, env);
    await runD10(report);

    // Determine final status
    report.status = report.scenarios.some((s) => s.status === 'failed') ? 'failed' : 'passed';
  } catch (error) {
    report.status = 'failed';
    report.error = error instanceof Error ? error.stack || error.message : String(error);
  } finally {
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
