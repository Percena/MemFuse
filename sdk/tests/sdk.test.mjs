/**
 * MemFuse SDK Tests — Phase 0 Validation
 *
 * Tests run against the compiled dist output.
 * Run: node --test tests/sdk.test.mjs
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { spawn } from 'node:child_process';
import { mkdtemp, readFile, access, writeFile, mkdir, chmod } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { Client } from '@modelcontextprotocol/sdk/client/index.js';
import { StdioClientTransport } from '@modelcontextprotocol/sdk/client/stdio.js';

// ─── 1. Brand rename verification ────────────────────────────────────

describe('Brand rename: rafs → mfs', () => {
  it('Config uses serverUrl not rafsUrl', async () => {
    const { loadConfig } = await import('../dist/shared/config.js');
    const config = loadConfig();
    assert.ok(config.serverUrl, 'config should have serverUrl');
    assert.equal(typeof config.serverUrl, 'string');
    assert.equal('rafsUrl' in config, false, 'config should not have rafsUrl');
  });

  it('PATHS constants use canonical Rust server paths', async () => {
    const { PATHS } = await import('../dist/hooks/platform-utils.js');
    assert.equal(PATHS.OBSERVE, '/sessions');
    assert.equal(PATHS.CONTEXT_RESOLVE, '/context/resolve');
    assert.equal(PATHS.CONSOLIDATE, '/sessions');
  });

  it('No RAFS env vars as primary in config', async () => {
    const fs = await import('node:fs');
    const configSrc = fs.readFileSync(
      new URL('../dist/shared/config.js', import.meta.url).pathname,
      'utf-8'
    );
    assert.ok(configSrc.includes('MEMFUSE_SERVER_URL'), 'MEMFUSE_SERVER_URL should be primary env var');
  });

  it('ContextDetailHandle type uses mfs_uri', async () => {
    const fs = await import('node:fs');
    const typesSrc = fs.readFileSync(
      new URL('../dist/client/types.d.ts', import.meta.url).pathname,
      'utf-8'
    );
    assert.ok(typesSrc.includes('mfs_uri'), 'types should use mfs_uri not rafs_uri');
    assert.equal(typesSrc.includes('rafs_uri'), false, 'types should NOT contain rafs_uri');
  });
});

// ─── 1b. Shared payload utilities ─────────────────────────────────────

describe('Shared payload utilities', () => {
  it('toArray returns arrays and normalizes non-arrays to empty arrays', async () => {
    const { toArray } = await import('../dist/shared/utils.js');
    const input = [{ id: 'a' }];

    assert.equal(toArray(input), input);
    assert.deepEqual(toArray(null), []);
    assert.deepEqual(toArray({ id: 'a' }), []);
    assert.deepEqual(toArray('not-array'), []);
  });
});

// ─── 2. Hook platform detection ────────────────────────────────────────

describe('Hook platform detection', () => {
  it('detects Claude Code platform for standard input', async () => {
    const { detectPlatform } = await import('../dist/hooks/platform-utils.js');
    assert.equal(detectPlatform({
      hook_event_name: 'PostToolUse',
      tool_name: 'Read',
      tool_input: { file_path: '/some/file.ts' },
      session_id: 'session-123',
    }), 'claude-code');
  });

  it('detects Codex platform for Bash command input', async () => {
    const { detectPlatform } = await import('../dist/hooks/platform-utils.js');
    assert.equal(detectPlatform({
      hook_event_name: 'PostToolUse',
      tool_name: 'Bash',
      tool_input: { command: 'npm test' },
    }), 'codex');
  });

  it('detects Codex from SessionStart with source string', async () => {
    const { detectPlatform } = await import('../dist/hooks/platform-utils.js');
    assert.equal(detectPlatform({
      hook_event_name: 'SessionStart',
      source: 'codex-cli',
      transcript_path: '/path/to/transcript',
    }), 'codex');
  });
});

// ─── 3. Input adaptation ────────────────────────────────────────────────

describe('Input adaptation', () => {
  it('adapts Claude Code input correctly', async () => {
    const { adaptInput } = await import('../dist/hooks/platform-utils.js');
    const adapted = adaptInput({
      session_id: 'session-1',
      tool_name: 'Read',
      tool_input: { file_path: '/test.ts' },
      tool_output: 'file content',
      user_id: 'user-1',
    });
    assert.equal(adapted._platform, 'claude-code');
    assert.equal(adapted.tool_name, 'Read');
    assert.equal(adapted.tool_input, '{"file_path":"/test.ts"}');
    assert.equal(adapted.tool_output, 'file content');
    assert.equal(adapted.user_id, 'user-1');
  });

  it('adapts Codex input correctly', async () => {
    const { adaptInput } = await import('../dist/hooks/platform-utils.js');
    const adapted = adaptInput({
      session_id: 'sess-2',
      tool_name: 'Bash',
      tool_input: { command: 'npm test' },
      tool_response: 'all tests passed',
    });
    assert.equal(adapted._platform, 'codex');
    assert.equal(adapted.tool_name, 'Bash');
    assert.equal(adapted.tool_input, 'npm test');
    assert.equal(adapted.tool_output, 'all tests passed');
  });

  it('prefers MEMFUSE_SESSION_ID over Codex hook payload session_id', async () => {
    const { adaptInput } = await import('../dist/hooks/platform-utils.js');
    const previous = process.env.MEMFUSE_SESSION_ID;
    process.env.MEMFUSE_SESSION_ID = 'env-session';
    try {
      const adapted = adaptInput({
        session_id: 'payload-session',
        tool_name: 'Bash',
        tool_input: { command: 'npm test' },
      });
      assert.equal(adapted._platform, 'codex');
      assert.equal(adapted.session_id, 'env-session');
    } finally {
      if (previous === undefined) delete process.env.MEMFUSE_SESSION_ID;
      else process.env.MEMFUSE_SESSION_ID = previous;
    }
  });
});

// ─── 4. Privacy stripping ────────────────────────────────────────────────

describe('Privacy stripping', () => {
  it('strips <private> blocks', async () => {
    const { stripPrivate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(stripPrivate('Public <private>secret</private> more'), 'Public  more');
  });

  it('strips multiline <private> blocks', async () => {
    const { stripPrivate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(stripPrivate('Before <private>\nline1\nline2\n</private> After'), 'Before  After');
  });

  it('strips multiple <private> blocks', async () => {
    const { stripPrivate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(stripPrivate('A <private>1</private> B <private>2</private> C'), 'A  B  C');
  });

  it('strips nested <private> blocks without leaking inner content', async () => {
    const { stripPrivate } = await import('../dist/hooks/platform-utils.js');
    const stripped = stripPrivate('Outer <private>inner <private>nested secret</private> still private</private> end');
    assert.equal(stripped, 'Outer  end');
    assert.equal(stripped.includes('nested secret'), false);
    assert.equal(stripped.includes('still private'), false);
    assert.equal(stripped.includes('</private>'), false);
  });

  it('handles empty/null content', async () => {
    const { stripPrivate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(stripPrivate(''), '');
  });

  it('sanitizes common secret token patterns', async () => {
    const { sanitizeSecrets } = await import('../dist/hooks/platform-utils.js');
    const input = 'sk-proj-abcdefghi Bearer abcdefghijk ghp_abcdefghijk cr_abcdefghijk';
    const output = sanitizeSecrets(input);
    assert.equal(output.includes('sk-proj-abcdefghi'), false);
    assert.equal(output.includes('Bearer abcdefghijk'), false);
    assert.equal(output.includes('ghp_abcdefghijk'), false);
    assert.equal(output.includes('cr_abcdefghijk'), false);
    assert.match(output, /sk-\[REDACTED\]/);
    assert.match(output, /Bearer \[REDACTED\]/);
    assert.match(output, /ghp_\[REDACTED\]/);
    assert.match(output, /cr_\[REDACTED\]/);
  });
});

// ─── 5. Error classification ────────────────────────────────────────────

describe('Error classification', () => {
  it('classifies ECONNREFUSED as degradable', async () => {
    const { isDegradableError } = await import('../dist/hooks/platform-utils.js');
    assert.equal(isDegradableError(new Error('ECONNREFUSED')), true);
  });

  it('classifies timeout as degradable', async () => {
    const { isDegradableError } = await import('../dist/hooks/platform-utils.js');
    assert.equal(isDegradableError(new Error('Request timeout')), true);
  });

  it('classifies 5xx as degradable', async () => {
    const { isDegradableError } = await import('../dist/hooks/platform-utils.js');
    assert.equal(isDegradableError(new Error('Server returned 503')), true);
  });

  it('classifies 4xx as non-degradable', async () => {
    const { isDegradableError } = await import('../dist/hooks/platform-utils.js');
    assert.equal(isDegradableError(new Error('Server returned 400')), false);
  });

  it('classifies null/undefined as non-degradable', async () => {
    const { isDegradableError } = await import('../dist/hooks/platform-utils.js');
    assert.equal(isDegradableError(null), false);
  });
});

// ─── 6. String truncation ────────────────────────────────────────────────

describe('String truncation', () => {
  it('truncates long strings', async () => {
    const { truncate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(truncate('a'.repeat(100), 50), 'a'.repeat(50) + '\n... [truncated]');
  });

  it('does not truncate short strings', async () => {
    const { truncate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(truncate('hello', 50), 'hello');
  });

  it('handles empty strings', async () => {
    const { truncate } = await import('../dist/hooks/platform-utils.js');
    assert.equal(truncate('', 50), '');
  });
});

// ─── 7. Output format adaptation ────────────────────────────────────────

describe('Output format', () => {
  it('Claude Code: plain text', async () => {
    const { formatOutput } = await import('../dist/hooks/platform-utils.js');
    assert.equal(formatOutput('claude-code', 'SessionStart', 'Hello'), 'Hello');
  });

  it('Codex: JSON hook wrapper', async () => {
    const { formatOutput } = await import('../dist/hooks/platform-utils.js');
    const parsed = JSON.parse(formatOutput('codex', 'SessionStart', 'Hello'));
    assert.equal(parsed.hookSpecificOutput.hookEventName, 'SessionStart');
    assert.equal(parsed.hookSpecificOutput.additionalContext, 'Hello');
  });
});

// ─── 8. HTTP Client ────────────────────────────────────────────────────────

describe('HTTP Client', () => {
  it('creates HttpClient with default options', async () => {
    const { HttpClient } = await import('../dist/client/http.js');
    const client = new HttpClient({ baseUrl: 'http://localhost:8720' });
    assert.ok(client);
    assert.equal(typeof client.get, 'function');
    assert.equal(typeof client.post, 'function');
    assert.equal(typeof client.put, 'function');
    assert.equal(typeof client.delete, 'function');
  });

  it('MemFuseHttpError has correct properties', async () => {
    const { MemFuseHttpError } = await import('../dist/client/http.js');
    const error = new MemFuseHttpError(500, { error: 'internal' }, true);
    assert.equal(error.status, 500);
    assert.equal(error.retryable, true);
    assert.equal(error.name, 'MemFuseHttpError');
  });

  it('runtime client resolveContext sends canonical payload and normalizes flat response', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        requests.push(JSON.parse(body || '{}'));
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          sections: {
            current_facts: [{ fact_id: 'fact-1', predicate: 'location.current_city', display_value: 'Tokyo', confidence: 0.95 }],
            recent_updates: [{ turn_id: 'turn-1', role: 'assistant', content: 'Confirmed the OAuth rotation workflow.' }],
            relevant_history: [{ episode_id: 'episode-1', summary: 'Investigated OAuth rotation workflow', salience: 0.91 }],
          },
          artifacts: { cross_session_briefs: [] },
          detail_handles: ['episode-1'],
          rendered_markdown: '[Current Facts]\n- Tokyo',
        }));
      });
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { MemFuseRuntimeClient } = await import('../dist/client/runtime-client.js');
    const client = new MemFuseRuntimeClient({ baseUrl: `http://127.0.0.1:${port}` });
    const result = await client.resolveContext({
      user_id: 'alice',
      session_id: 'session-1',
      query: 'oauth rotation',
      token_budget: 1200,
    });

    server.close();

    assert.deepEqual(requests, [{
      user_id: 'alice',
      session_id: 'session-1',
      query: 'oauth rotation',
      token_budget: 1200,
    }]);
    assert.equal(result.sections.current_facts[0].display_value, 'Tokyo');
    assert.equal(result.detail_handles[0], 'episode-1');
    assert.equal(result.rendered_markdown, '[Current Facts]\n- Tokyo');
  });

  it('ops client exposes health and system operability endpoints', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          status: 'alive',
          version: '0.1.0',
          summary_provider: 'deterministic',
          embedding_provider: 'deterministic',
        }));
        return;
      }

      if (req.url === '/ready') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          status: 'ready',
          checks: { workspace: 'ok', metadata: 'ok' },
        }));
        return;
      }

      if (req.url === '/system/status') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          workspace_root: '/tmp/memfuse',
          resources: { total: 1, ready: 1, processing: 0, failed: 0 },
          metadata_tasks: { total: 2, pending: 0, running: 1, completed: 1, failed: 0 },
          session_tasks: { total: 3, pending: 1, running: 0, completed: 2, failed: 0 },
          snapshots_total: 4,
          runtime: {
            status: 'ok',
            retrieval_cache: { entries: 5, builds: 6, hits: 7, invalidations: 8 },
          },
        }));
        return;
      }

      if (req.url === '/system/observer') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          runtime: {
            summary_provider: 'deterministic',
            embedding_provider: 'deterministic',
            retrieval_cache: { entries: 1, builds: 2, hits: 3, invalidations: 4 },
          },
          semantic: {
            total_documents: 10,
            resource_documents: 3,
            memory_documents: 4,
            skill_documents: 3,
            embedding_dimension: 1024,
          },
        }));
        return;
      }

      if (req.url === '/watch-service/status') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          running: true,
          poll_ms: 1000,
          started_at_ms: 1,
          stopped_at_ms: null,
          last_tick_at_ms: 2,
          total_ticks: 3,
          total_runs: 4,
          last_run_count: 5,
        }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { MemFuseOpsClient } = await import('../dist/client/ops-client.js');
    const client = new MemFuseOpsClient({ baseUrl: `http://127.0.0.1:${port}` });

    const [health, ready, status, observer, watch] = await Promise.all([
      client.getHealth(),
      client.getReady(),
      client.getSystemStatus(),
      client.getObserverStatus(),
      client.getWatchServiceStatus(),
    ]);

    server.close();

    assert.equal(health.status, 'alive');
    assert.equal(ready.status, 'ready');
    assert.equal(status.resources.ready, 1);
    assert.equal(status.runtime.retrieval_cache.hits, 7);
    assert.equal(observer.semantic.embedding_dimension, 1024);
    assert.equal(watch.running, true);
  });

  it('CodexMemoryAdapter prepareRead provides a read-time hint fallback', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          const parsed = JSON.parse(body || '{}');
          assert.equal(parsed.query, 'src/auth.ts');
          assert.equal(parsed.session_id, undefined);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            total: 1,
            results: [{
              episode_id: 'episode-auth',
              summary: 'Investigated auth middleware ordering',
              score: 0.92,
            }],
          }));
        });
        return;
      }

      if (req.url === '/facts?user_id=alice') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          facts: [{
            fact_id: 'fact-auth',
            predicate: 'project.file_hint',
            display_value: 'src/auth.ts uses middleware sequencing',
            confidence: 0.91,
          }],
        }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { MemFuseRuntimeClient } = await import('../dist/client/runtime-client.js');
    const { CodexMemoryAdapter } = await import('../dist/client/adapters/codex.js');
    const client = new MemFuseRuntimeClient({ baseUrl: `http://127.0.0.1:${port}` });
    const adapter = new CodexMemoryAdapter(client);

    const hint = await adapter.prepareRead({
      threadId: 'session-1',
      userId: 'alice',
      filePath: 'src/auth.ts',
    });

    server.close();

    assert.equal(hint.filePath, 'src/auth.ts');
    assert.equal(hint.relatedEpisodes.length, 1);
    assert.equal(hint.relatedFacts.length, 1);
    assert.match(hint.renderedText, /Investigated auth middleware ordering/);
    assert.match(hint.renderedText, /middleware sequencing/);
  });

  it('CodexMemoryAdapter exposes host-friendly session and read hint helpers', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/sessions/session-2/messages' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true, session_id: 'session-2' }));
        return;
      }

      if (req.url === '/context/resolve' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          sections: {
            current_facts: [{ fact_id: 'fact-1', predicate: 'project.active', display_value: 'Auth refactor', confidence: 0.95 }],
            recent_updates: [],
            relevant_history: [{ episode_id: 'ep-1', summary: 'Investigated auth guard ordering', salience: 0.8 }],
          },
          artifacts: {},
          detail_handles: ['ep-1'],
          rendered_markdown: '[Current Facts]\n- Auth refactor',
        }));
        return;
      }

      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          total: 1,
          results: [{ episode_id: 'ep-read', summary: 'Read auth.ts before fixing middleware order', score: 0.9 }],
        }));
        return;
      }

      if (req.url === '/facts?user_id=alice') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          facts: [{ fact_id: 'fact-read', predicate: 'project.file_hint', display_value: 'src/auth.ts contains guard ordering logic', confidence: 0.88 }],
        }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { MemFuseRuntimeClient } = await import('../dist/client/runtime-client.js');
    const { CodexMemoryAdapter } = await import('../dist/client/adapters/codex.js');
    const client = new MemFuseRuntimeClient({ baseUrl: `http://127.0.0.1:${port}` });
    const adapter = new CodexMemoryAdapter(client);

    const [sessionContext, readHint] = await Promise.all([
      adapter.prepareSessionContext({
        threadId: 'session-2',
        userId: 'alice',
        userMessage: 'Help me fix auth ordering',
        queryText: 'auth ordering',
      }),
      adapter.prepareReadHint({
        threadId: 'session-2',
        userId: 'alice',
        filePath: 'src/auth.ts',
      }),
    ]);

    server.close();

    assert.match(sessionContext, /Auth refactor/);
    assert.match(sessionContext, /Investigated auth guard ordering/);
    assert.match(readHint, /Read auth.ts before fixing middleware order/);
    assert.match(readHint, /guard ordering logic/);
  });

  it('GenericMemoryAdapter exposes generic host lifecycle methods from the client entrypoint', async () => {
    const seen = [];
    const server = http.createServer((req, res) => {
      if (req.url === '/sessions/session-generic/messages' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          seen.push(JSON.parse(body || '{}'));
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ ok: true, session_id: 'session-generic' }));
        });
        return;
      }

      if (req.url === '/context/resolve' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          sections: {
            current_facts: [{ fact_id: 'fact-generic', predicate: 'project.adapter', display_value: 'Generic adapter enabled', confidence: 0.94 }],
            recent_updates: [],
            relevant_history: [],
          },
          artifacts: {},
          detail_handles: [],
          rendered_markdown: '[Current Facts]\n- Generic adapter enabled',
        }));
        return;
      }

      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          total: 1,
          results: [{ episode_id: 'ep-generic', summary: 'Generic adapter checked src/generic.ts', score: 0.9 }],
        }));
        return;
      }

      if (req.url === '/facts?user_id=alice') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          facts: [{ fact_id: 'fact-read', predicate: 'project.file_hint', display_value: 'src/generic.ts has adapter integration', confidence: 0.88 }],
        }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { MemFuseRuntimeClient, GenericMemoryAdapter } = await import('../dist/client/index.js');
    const client = new MemFuseRuntimeClient({ baseUrl: `http://127.0.0.1:${port}` });
    const adapter = new GenericMemoryAdapter(client);

    try {
      const started = await adapter.startTurn({
        threadId: 'session-generic',
        userId: 'alice',
        userMessage: 'Start generic host turn',
        queryText: 'generic adapter',
        resourceId: 'repo-1',
      });
      const finished = await adapter.finishTurn({
        threadId: 'session-generic',
        userId: 'alice',
        assistantMessage: 'Finished generic host turn',
        resourceId: 'repo-1',
      });
      const read = await adapter.prepareRead({
        threadId: 'session-generic',
        userId: 'alice',
        filePath: 'src/generic.ts',
      });

      assert.equal(started.userTurn.session_id, 'session-generic');
      assert.match(started.context.rendered_markdown, /Generic adapter enabled/);
      assert.equal(finished.assistantTurn.session_id, 'session-generic');
      assert.equal(seen[0].role, 'user');
      assert.equal(seen[0].resource_id, 'repo-1');
      assert.equal(seen[1].role, 'assistant');
      assert.equal(seen[1].resource_id, 'repo-1');
      assert.match(read.renderedText, /Generic adapter checked src\/generic\.ts/);
      assert.match(read.renderedText, /adapter integration/);
    } finally {
      server.close();
    }
  });
});

// ─── 9. Setup installer ─────────────────────────────────────────────────

describe('Setup installer', () => {
  it('writes Claude Code hooks in canonical event-grouped schema without legacy array entries', async () => {
    const projectDir = await mkdtemp(join(tmpdir(), 'memfuse-claude-setup-'));
    const { runSetup } = await import('../dist/setup/install.js');
    const originalPath = process.env.PATH;
    process.env.PATH = '';

    try {
      await runSetup([
        '--platform=claude-code',
        `--project-dir=${projectDir}`,
        '--user-id=alice',
        '--server-url=http://127.0.0.1:9',
      ]);
    } finally {
      process.env.PATH = originalPath;
    }

    const settings = JSON.parse(await readFile(join(projectDir, '.claude', 'settings.local.json'), 'utf-8'));
    assert.equal(Array.isArray(settings.hooks), false, 'Claude Code hooks must be grouped by event, not a legacy array');

    for (const eventName of ['SessionStart', 'UserPromptSubmit', 'PreToolUse', 'PostToolUse', 'Stop', 'PreCompact', 'SessionEnd', 'Setup']) {
      assert.ok(Array.isArray(settings.hooks[eventName]), `${eventName} hook group should exist`);
      assert.ok(Array.isArray(settings.hooks[eventName][0].hooks), `${eventName} should contain command hooks`);
      assert.equal(settings.hooks[eventName][0].hooks[0].type, 'command');
      assert.match(settings.hooks[eventName][0].hooks[0].command, /--platform=claude-code/);
      assert.match(settings.hooks[eventName][0].hooks[0].command, /MEMFUSE_SERVER_URL="http:\/\/127\.0\.0\.1:9"/);
    }

    assert.equal(settings.hooks.PreToolUse[0].matcher, 'Read');
    await access(join(projectDir, '.claude-plugin', 'plugin.json'));
    await assert.rejects(access(join(projectDir, '.codex-plugin', 'plugin.json')));
  });

  it('installs the Codex plugin manifest into .codex-plugin and widens Codex hook matcher coverage', async () => {
    const projectDir = await mkdtemp(join(tmpdir(), 'memfuse-sdk-'));
    const codexHome = join(projectDir, 'codex-home');
    const { runSetup } = await import('../dist/setup/install.js');
    const originalPath = process.env.PATH;
    const originalCodexHome = process.env.CODEX_HOME;
    process.env.PATH = '';
    process.env.CODEX_HOME = codexHome;

    try {
      await runSetup([
        '--platform=codex',
        `--project-dir=${projectDir}`,
        '--user-id=alice',
        '--server-url=http://127.0.0.1:9',
      ]);
    } finally {
      process.env.PATH = originalPath;
      if (originalCodexHome === undefined) delete process.env.CODEX_HOME;
      else process.env.CODEX_HOME = originalCodexHome;
    }

    await access(join(projectDir, '.codex-plugin', 'plugin.json'));
    const hooksJson = JSON.parse(await readFile(join(projectDir, '.codex', 'hooks.json'), 'utf-8'));
    const sessionStartMatcher = hooksJson.hooks.SessionStart[0].matcher;
    const matcher = hooksJson.hooks.PostToolUse[0].matcher;
    const codexConfig = await readFile(join(codexHome, 'config.toml'), 'utf-8');

    assert.equal(sessionStartMatcher, 'startup|resume|clear|compact');
    assert.equal(matcher, 'Bash|Read|Edit|Write|MultiEdit|Glob|Grep|mcp__.*');
    assert.match(codexConfig, /\[features]\s+hooks = true/);
    assert.doesNotMatch(codexConfig, /codex_hooks/);
    assert.match(codexConfig, /:session_start:0:0/);
    assert.match(codexConfig, /:post_tool_use:0:0/);
    assert.match(codexConfig, /:stop:0:0/);
    assert.match(codexConfig, /trusted_hash = "sha256:[a-f0-9]{64}"/);
    await assert.rejects(access(join(projectDir, '.claude-plugin', 'plugin.json')));
  });

  it('reports detected Codex hook support when the Codex CLI exposes the hooks feature', async () => {
    const projectDir = await mkdtemp(join(tmpdir(), 'memfuse-codex-hooks-'));
    const binDir = join(projectDir, 'bin');
    const codexHome = join(projectDir, 'codex-home');
    await mkdir(binDir, { recursive: true });
    const fakeCodex = join(binDir, 'codex');
    await writeFile(fakeCodex, `#!/usr/bin/env node
const args = process.argv.slice(2);
if (args[0] === 'features' && args[1] === 'list') {
  console.log('hooks stable true');
  process.exit(0);
}
process.exit(1);
`, 'utf-8');
    await chmod(fakeCodex, 0o755);

    const { runSetup } = await import('../dist/setup/install.js');
    const originalPath = process.env.PATH;
    const originalCodexHome = process.env.CODEX_HOME;
    const originalLog = console.log;
    const logs = [];
    process.env.PATH = `${binDir}:${originalPath || ''}`;
    process.env.CODEX_HOME = codexHome;
    console.log = (...args) => { logs.push(args.join(' ')); };

    try {
      await runSetup([
        '--platform=codex',
        `--project-dir=${projectDir}`,
        '--user-id=alice',
        '--server-url=http://127.0.0.1:9',
      ]);
    } finally {
      console.log = originalLog;
      process.env.PATH = originalPath;
      if (originalCodexHome === undefined) delete process.env.CODEX_HOME;
      else process.env.CODEX_HOME = originalCodexHome;
    }

    const output = logs.join('\n');
    assert.match(output, /Codex hooks support detected/);
    assert.doesNotMatch(output, /require a Codex build with hooks support/);
  });
});

describe('Documentation consistency', () => {
  it('does not document 8720 as the independent service default after port unification', async (t) => {
    // This doc lives in the monorepo, not in the published @percena/memfuse
    // package. Skip rather than fail when the suite runs outside the repo tree.
    const docPath = new URL('../../docs/architecture.md', import.meta.url);
    let architecture;
    try {
      architecture = await readFile(docPath, 'utf-8');
    } catch (err) {
      if (err.code === 'ENOENT') { t.skip('docs/architecture.md not present outside monorepo'); return; }
      throw err;
    }
    assert.doesNotMatch(architecture, /独立服务默认 `http:\/\/127\.0\.0\.1:8720`/);
    assert.match(architecture, /`MEMFUSE_SERVER_URL` 或内置默认 `http:\/\/127\.0\.0\.1:18720`/);
  });
});

// ─── 10. MCP DIG surface ───────────────────────────────────────────────

describe('MCP DIG tools', () => {
  it('registers DIG tools and proxies list/read/grep through the MemFuse server', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      requests.push(req.url || '');

      if (req.url === '/ls?uri=mfs%3A%2F%2Fresources%2Flocalfs%2Fdocs') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify([{ name: 'api.md', is_dir: false }, { name: 'guides', is_dir: true }]));
        return;
      }

      if (req.url === '/read?uri=mfs%3A%2F%2Fresources%2Flocalfs%2Fdocs%2Fapi.md') {
        res.writeHead(200, { 'Content-Type': 'text/plain' });
        res.end('# API\n\nAuth endpoints');
        return;
      }

      if (req.url === '/grep?query=auth&target=mfs%3A%2F%2Fresources%2Flocalfs%2Fdocs') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          resources: [{ uri: 'mfs://resources/localfs/docs/api.md', excerpt: 'Authentication endpoints', score: 0.88 }],
          memories: [],
          skills: [],
        }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const client = new Client({ name: 'memfuse-sdk-test', version: '1.0.0' });
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: ['bin/memfuse-mcp.cjs'],
      cwd: new URL('..', import.meta.url).pathname,
      env: {
        ...process.env,
        MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
        MEMFUSE_USER_ID: 'alice',
      },
      stderr: 'pipe',
    });

    await client.connect(transport);

    const tools = await client.listTools();
    const toolNames = tools.tools.map(tool => tool.name);
    for (const name of ['ls', 'read', 'abstract', 'overview', 'glob', 'grep']) {
      assert.ok(toolNames.includes(name), `expected DIG tool ${name} to be registered`);
    }
    assert.ok(!toolNames.includes('skill_suggestions'), 'removed skill evolution tool should not be registered');
    assert.ok(!toolNames.includes('skill_apply_patch'), 'removed skill evolution tool should not be registered');

    const lsResult = await client.callTool({
      name: 'ls',
      arguments: { uri: 'mfs://resources/localfs/docs' },
    });
    const readResult = await client.callTool({
      name: 'read',
      arguments: { uri: 'mfs://resources/localfs/docs/api.md' },
    });
    const grepResult = await client.callTool({
      name: 'grep',
      arguments: { query: 'auth', target: 'mfs://resources/localfs/docs' },
    });

    await client.close();
    server.close();

    assert.match(lsResult.content[0].text, /api\.md/);
    assert.match(lsResult.content[0].text, /guides/);
    assert.match(lsResult.content[0].text, /📁/);  // directory indicator
    assert.match(lsResult.content[0].text, /mfs:\/\/resources\/localfs\/docs\/api\.md/);
    assert.match(readResult.content[0].text, /Auth endpoints/);
    assert.match(grepResult.content[0].text, /mfs:\/\/resources\/localfs\/docs\/api\.md/);
    assert.deepEqual(requests, [
      '/ls?uri=mfs%3A%2F%2Fresources%2Flocalfs%2Fdocs',
      '/read?uri=mfs%3A%2F%2Fresources%2Flocalfs%2Fdocs%2Fapi.md',
      '/grep?query=auth&target=mfs%3A%2F%2Fresources%2Flocalfs%2Fdocs',
    ]);
  });

  it('registers relation tools and proxies link/list through the MemFuse server', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        requests.push({ method: req.method, url: req.url, body: body ? JSON.parse(body) : null });

        if (req.method === 'POST' && req.url === '/relations') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ ok: true }));
          return;
        }

        if (req.method === 'GET' && req.url === '/relations?uri=mfs%3A%2F%2Fdocs%2Fa&limit=5') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            relations: [{
              direction: 'outbound',
              peer_uri: 'mfs://docs/b',
              relation_type: 'references',
            }],
          }));
          return;
        }

        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'not found' }));
      });
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const client = new Client({ name: 'memfuse-sdk-test', version: '1.0.0' });
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: ['bin/memfuse-mcp.cjs'],
      cwd: new URL('..', import.meta.url).pathname,
      env: {
        ...process.env,
        MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
        MEMFUSE_USER_ID: 'alice',
      },
      stderr: 'pipe',
    });

    await client.connect(transport);

    try {
      const tools = await client.listTools();
      const toolNames = tools.tools.map(tool => tool.name);
      assert.ok(toolNames.includes('link_relations'));
      assert.ok(toolNames.includes('list_relations'));

      const linkResult = await client.callTool({
        name: 'link_relations',
        arguments: {
          from_uri: 'mfs://docs/a',
          to_uri: 'mfs://docs/b',
          relation_type: 'references',
        },
      });
      const listResult = await client.callTool({
        name: 'list_relations',
        arguments: { uri: 'mfs://docs/a', limit: 5 },
      });

      assert.match(linkResult.content[0].text, /Relation linked/);
      assert.match(listResult.content[0].text, /mfs:\/\/docs\/b/);
      assert.deepEqual(requests, [
        {
          method: 'POST',
          url: '/relations',
          body: {
            from_uri: 'mfs://docs/a',
            to_uri: 'mfs://docs/b',
            relation_type: 'references',
          },
        },
        {
          method: 'GET',
          url: '/relations?uri=mfs%3A%2F%2Fdocs%2Fa&limit=5',
          body: null,
        },
      ]);
    } finally {
      await client.close();
      server.close();
    }
  });

  it('registers repo-intelligence tools and proxies PRD-shaped HTTP calls', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        requests.push({ method: req.method, url: req.url, body: body ? JSON.parse(body) : null });

        if (req.method === 'GET' && req.url === '/manifest/get?repo_id=symphony-gh') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            status: 'ok',
            data: {
              repo_identity: {
                repo_id: 'symphony-gh',
                resource_uri: 'mfs://resources/localfs/symphony-gh/MANIFEST.yaml',
                default_branch: 'main',
                primary_languages: ['elixir'],
              },
              manifest_yaml_path: '/tmp/MANIFEST.yaml',
            },
          }));
          return;
        }

        if (req.method === 'GET' && req.url === '/canvas/query?repo_id=symphony-gh&component=Runner&type=structural') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            status: 'ok',
            data: {
              nodes: [{ id: 'n1', node_type: 'module', name: 'Runner' }],
              edges: [],
              overlays: [],
              conflicts: [],
            },
          }));
          return;
        }

        if (req.method === 'POST' && req.url === '/overlay/propose') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ status: 'ok', data: { overlay_id: 'overlay-1', status: 'proposed' } }));
          return;
        }

        if (req.method === 'POST' && req.url === '/overlay/report_conflict') {
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            status: 'ok',
            data: {
              conflict_id: 'conflict-1',
              has_conflict: true,
              requires_human_review: true,
              overlap_nodes: ['n1'],
              overlap_edges: [],
            },
          }));
          return;
        }

        res.writeHead(404, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ error: 'not found' }));
      });
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const client = new Client({ name: 'memfuse-sdk-test', version: '1.0.0' });
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: ['bin/memfuse-mcp.cjs'],
      cwd: new URL('..', import.meta.url).pathname,
      env: {
        ...process.env,
        MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
        MEMFUSE_USER_ID: 'alice',
      },
      stderr: 'pipe',
    });

    await client.connect(transport);

    try {
      const tools = await client.listTools();
      const toolNames = tools.tools.map(tool => tool.name);
      for (const name of ['get_repo_manifest', 'query_canvas', 'propose_active_overlay', 'report_conflict']) {
        assert.ok(toolNames.includes(name), `expected repo-intelligence tool ${name} to be registered`);
      }

      const manifest = await client.callTool({
        name: 'get_repo_manifest',
        arguments: { repo_id: 'symphony-gh' },
      });
      const canvas = await client.callTool({
        name: 'query_canvas',
        arguments: { repo_id: 'symphony-gh', component: 'Runner', type: 'structural' },
      });
      const overlay = await client.callTool({
        name: 'propose_active_overlay',
        arguments: {
          repo_id: 'symphony-gh',
          tracker: 'github_projects',
          tracker_content_id: 'I_kwDO_1',
          tracker_identifier: 'owner/repo#1',
          overlay_type: 'planned_change',
          affected_nodes: ['n1'],
          content_json: { summary: 'change runner' },
          author: 'alice',
        },
      });
      const conflict = await client.callTool({
        name: 'report_conflict',
        arguments: {
          repo_id: 'symphony-gh',
          overlay_id_1: 'overlay-1',
          overlay_id_2: 'overlay-2',
          conflict_description: 'same node',
        },
      });

      assert.match(manifest.content[0].text, /symphony-gh/);
      assert.match(canvas.content[0].text, /Runner/);
      assert.match(overlay.content[0].text, /overlay-1/);
      assert.match(conflict.content[0].text, /requires_human_review: true/);
      assert.deepEqual(requests.map(r => r.url), [
        '/manifest/get?repo_id=symphony-gh',
        '/canvas/query?repo_id=symphony-gh&component=Runner&type=structural',
        '/overlay/propose',
        '/overlay/report_conflict',
      ]);
      assert.equal(requests[2].body.author, 'alice');
      assert.deepEqual(requests[2].body.content_json, { summary: 'change runner' });
      assert.deepEqual(requests[3].body, {
        repo_id: 'symphony-gh',
        overlay_id_1: 'overlay-1',
        overlay_id_2: 'overlay-2',
        conflict_description: 'same node',
      });
    } finally {
      await client.close();
      server.close();
    }
  });

  it('search_memories searches across sessions by default even when MCP has a current session', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          requests.push(JSON.parse(body || '{}'));
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            total: 1,
            results: [{
              episode_id: 'ep-cross-session',
              session_id: 'previous-session',
              summary: 'Prior context injection work in memory.rs',
              score: 1,
            }],
          }));
        });
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const client = new Client({ name: 'memfuse-sdk-test', version: '1.0.0' });
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: ['bin/memfuse-mcp.cjs'],
      cwd: new URL('..', import.meta.url).pathname,
      env: {
        ...process.env,
        MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
        MEMFUSE_USER_ID: 'alice',
        MEMFUSE_SESSION_ID: 'current-session',
      },
      stderr: 'pipe',
    });

    await client.connect(transport);

    const result = await client.callTool({
      name: 'search_memories',
      arguments: { query: 'memory.rs context injection', limit: 5 },
    });

    try {
      assert.match(result.content[0].text, /ep-cross-session/);
      assert.deepEqual(requests, [{
        user_id: 'alice',
        query: 'memory.rs context injection',
        limit: 5,
        strategy: 'precision',
      }]);
    } finally {
      await client.close();
      server.close();
    }
  });

  it('registers session management tools and proxies list/get/delete', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      requests.push({ method: req.method, url: req.url });

      if (req.method === 'GET' && req.url === '/sessions?limit=2') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ session_id: 'session-a', status: 'active' }],
          sessions: [{ session_id: 'session-a', status: 'active' }],
          total_count: 1,
          count: 1,
          limit: 2,
          next_cursor: null,
        }));
        return;
      }

      if (req.method === 'GET' && req.url === '/sessions/session-a') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ session_id: 'session-a', status: 'active' }));
        return;
      }

      if (req.method === 'DELETE' && req.url === '/sessions/session-a') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ deleted: true }));
        return;
      }

      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const client = new Client({ name: 'memfuse-sdk-test', version: '1.0.0' });
    const transport = new StdioClientTransport({
      command: process.execPath,
      args: ['bin/memfuse-mcp.cjs'],
      cwd: new URL('..', import.meta.url).pathname,
      env: {
        ...process.env,
        MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
        MEMFUSE_USER_ID: 'alice',
      },
      stderr: 'pipe',
    });

    await client.connect(transport);

    try {
      const tools = await client.listTools();
      const toolNames = tools.tools.map(tool => tool.name);
      for (const name of ['session_list', 'session_get', 'session_delete']) {
        assert.ok(toolNames.includes(name), `expected MCP tool ${name}`);
      }

      const listResult = await client.callTool({
        name: 'session_list',
        arguments: { limit: 2 },
      });
      const getResult = await client.callTool({
        name: 'session_get',
        arguments: { session_id: 'session-a' },
      });
      const deleteResult = await client.callTool({
        name: 'session_delete',
        arguments: { session_id: 'session-a' },
      });

      assert.match(listResult.content[0].text, /session-a/);
      assert.match(getResult.content[0].text, /session-a/);
      assert.match(deleteResult.content[0].text, /deleted/i);
      assert.deepEqual(requests, [
        { method: 'GET', url: '/sessions?limit=2' },
        { method: 'GET', url: '/sessions/session-a' },
        { method: 'DELETE', url: '/sessions/session-a' },
      ]);
    } finally {
      await client.close();
      server.close();
    }
  });
});

// ─── 9. Skills loader ────────────────────────────────────────────────────────

describe('Skills loader', () => {
  it('loads the unified skill name', async () => {
    const { SKILL_NAMES } = await import('../dist/skills/loader.js');
    assert.deepEqual(SKILL_NAMES, ['memfuse']);
  });

  it('loads skill content for memfuse', async () => {
    const { loadSkill } = await import('../dist/skills/loader.js');
    const content = loadSkill('memfuse');
    assert.ok(content.length > 0);
    assert.ok(content.includes('memfuse'));
  });

  it('skill has YAML frontmatter', async () => {
    const { loadSkill } = await import('../dist/skills/loader.js');
    const content = loadSkill('memfuse');
    assert.ok(content.startsWith('---'));
    assert.ok(content.includes('name: memfuse'));
    assert.ok(content.includes('description:'));
  });

  it('installs bundled MemFuse skill references', async () => {
    const { installSkill } = await import('../dist/skills/loader.js');
    const target = await mkdtemp(join(tmpdir(), 'memfuse-skill-install-'));

    installSkill('memfuse', target);

    await access(join(target, 'memfuse', 'SKILL.md'));
    await access(join(target, 'memfuse', 'references', 'commands.md'));
  });
});

// ─── 10. Config ────────────────────────────────────────────────────────────────

describe('Config', () => {
  it('loads default config', async () => {
    const { loadConfig } = await import('../dist/shared/config.js');
    const config = loadConfig();
    assert.equal(config.serverUrl, 'http://127.0.0.1:18720');
    assert.equal(config.userId, process.env.MEMFUSE_USER_ID || process.env.USER || 'default');
    assert.equal(config.sessionId, '');
  });

  it('respects MEMFUSE_SERVER_URL env var', async () => {
    const origUrl = process.env.MEMFUSE_SERVER_URL;
    process.env.MEMFUSE_SERVER_URL = 'http://custom:9999';
    // Re-import to get fresh module evaluation
    const freshModule = await import('../dist/shared/config.js?update=' + Date.now());
    const config = freshModule.loadConfig();
    assert.equal(config.serverUrl, 'http://custom:9999');
    process.env.MEMFUSE_SERVER_URL = origUrl;
  });
});

describe('Client render helpers', () => {
  it('renderContextText uses normalized context sections', async () => {
    const { renderContextText } = await import('../dist/client/render.js');
    const rendered = renderContextText({
      sections: {
        current_facts: [{ fact_id: 'fact-1', predicate: 'location.current_city', display_value: 'Tokyo', confidence: 0.95 }],
        recent_updates: [{ turn_id: 'turn-1', role: 'assistant', content: 'Confirmed the OAuth rotation workflow.' }],
        relevant_history: [{ episode_id: 'episode-1', summary: 'Investigated OAuth rotation workflow', salience: 0.91 }],
      },
      artifacts: {},
      detail_handles: [],
      rendered_markdown: '[Current Facts]\n- Tokyo',
    });
    assert.match(rendered, /\[Current Facts\]/);
    assert.match(rendered, /Tokyo/);
    assert.match(rendered, /Investigated OAuth rotation workflow/);
  });
});

describe('Shared HTTP helper', () => {
  it('throws on non-2xx JSON responses', async () => {
    const server = http.createServer((_req, res) => {
      res.writeHead(422, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'bad request' }));
    });
    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const originalUrl = process.env.MEMFUSE_SERVER_URL;
    process.env.MEMFUSE_SERVER_URL = `http://127.0.0.1:${port}`;
    const httpModule = await import('../dist/shared/http.js?update=' + Date.now());

    await assert.rejects(
      () => httpModule.callServer('POST', '/context/resolve', { query: 'x' }),
      /Server returned 422/,
    );

    process.env.MEMFUSE_SERVER_URL = originalUrl;
    server.close();
  });
});

// ─── 11. Hook integration ───────────────────────────────────────────────

describe('SessionStart hook', () => {
  it('setup hook gives service start and development server hints when offline', async () => {
    const child = spawn(
      process.execPath,
      ['dist/hooks/setup.js'],
      {
        cwd: new URL('..', import.meta.url),
        env: {
          ...process.env,
          MEMFUSE_SERVER_URL: 'http://127.0.0.1:9',
        },
        stdio: ['pipe', 'pipe', 'pipe'],
      },
    );

    let stderr = '';
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.stdin.end('{}');

    const exitCode = await new Promise(resolve => child.on('close', resolve));
    assert.equal(exitCode, 0, `stderr:\n${stderr}`);
    assert.match(stderr, /memfuse service start/);
    assert.match(stderr, /\.\/run-server\.sh/);
  });

  it('sends canonical resolve request and renders flat context response', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      if (req.method !== 'POST' || req.url !== '/context/resolve') {
        res.writeHead(404);
        res.end('not found');
        return;
      }

      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        const parsed = JSON.parse(body || '{}');
        requests.push(parsed);
        const valid = parsed.session_id === 'session-hook'
          && parsed.token_budget === 1500
          && typeof parsed.query === 'string'
          && parsed.query.trim().length > 0;
        if (!valid) {
          res.writeHead(422, { 'Content-Type': 'text/plain' });
          res.end('bad request');
          return;
        }

        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          sections: {
            current_facts: [{ subject: 'user', predicate: 'location.current_city', display_value: 'Tokyo', confidence: 0.95 }],
            recent_updates: [{ role: 'assistant', content: 'Confirmed the OAuth rotation workflow.' }],
            relevant_history: [{ episode_id: 'episode-1', summary: 'Investigated OAuth rotation workflow', score: 0.91 }],
          },
          artifacts: {},
          detail_handles: [],
          rendered_markdown: '[Current Facts]\n- Tokyo',
        }));
      });
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const child = spawn(
      process.execPath,
      ['dist/hooks/session-start.js'],
      {
        cwd: new URL('..', import.meta.url),
        env: {
          ...process.env,
          MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
          MEMFUSE_USER_ID: 'alice',
        },
        stdio: ['pipe', 'pipe', 'pipe'],
      },
    );

    let stdout = '';
    let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.stdin.end(JSON.stringify({
      hook_event_name: 'SessionStart',
      session_id: 'session-hook',
      user_id: 'alice',
      source: 'codex-cli',
      transcript_path: '/tmp/transcript',
    }));

    const exitCode = await new Promise(resolve => child.on('close', resolve));
    try {
      assert.equal(exitCode, 0, `stderr:\n${stderr}`);
      assert.equal(requests.length, 1);
      assert.equal(requests[0].user_id, 'alice');
      assert.equal(requests[0].session_id, 'session-hook');
      assert.equal(requests[0].token_budget, 1500);
      assert.match(requests[0].query, /session continuity/);
      assert.match(stdout, /Tokyo/);
      assert.match(stdout, /OAuth rotation workflow/);
    } finally {
      server.close();
    }
  });
});

describe('PostToolUse hook', () => {
  it('stores observations with a single request and relies on server-side session upsert', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        requests.push({ method: req.method, url: req.url, body: body ? JSON.parse(body) : {} });
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true, session_id: 'session-hook' }));
      });
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const child = spawn(
      process.execPath,
      ['dist/hooks/post-tool-use.js'],
      {
        cwd: new URL('..', import.meta.url),
        env: {
          ...process.env,
          MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
          MEMFUSE_USER_ID: 'alice',
          MEMFUSE_SESSION_ID: 'session-hook',
        },
        stdio: ['pipe', 'pipe', 'pipe'],
      },
    );

    let stderr = '';
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.stdin.end(JSON.stringify({
      hook_event_name: 'PostToolUse',
      session_id: 'payload-session',
      tool_name: 'Bash',
      tool_input: { command: 'printf E2E' },
      tool_response: 'E2E',
    }));

    const exitCode = await new Promise(resolve => child.on('close', resolve));
    try {
      assert.equal(exitCode, 0, `stderr:\n${stderr}`);
      assert.equal(requests.length, 1);
      assert.equal(requests[0].method, 'POST');
      assert.equal(requests[0].url, '/sessions/session-hook/observations');
      assert.equal(requests[0].body.tool_name, 'Bash');
    } finally {
      server.close();
    }
  });

  it('sanitizes secret-like tokens before storing observations', async () => {
    const requests = [];
    const server = http.createServer((req, res) => {
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        requests.push({ method: req.method, url: req.url, body: body ? JSON.parse(body) : {} });
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true, session_id: 'session-secret' }));
      });
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const child = spawn(
      process.execPath,
      ['dist/hooks/post-tool-use.js'],
      {
        cwd: new URL('..', import.meta.url),
        env: {
          ...process.env,
          MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
          MEMFUSE_USER_ID: 'alice',
          MEMFUSE_SESSION_ID: 'session-secret',
        },
        stdio: ['pipe', 'pipe', 'pipe'],
      },
    );

    let stderr = '';
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.stdin.end(JSON.stringify({
      hook_event_name: 'PostToolUse',
      session_id: 'payload-session',
      tool_name: 'Bash',
      tool_input: { command: 'curl -H "Authorization: Bearer abcdefghijk" https://api.example.test' },
      tool_response: 'failed with sk-proj-abcdefghi and ghp_abcdefghijk',
    }));

    const exitCode = await new Promise(resolve => child.on('close', resolve));
    try {
      assert.equal(exitCode, 0, `stderr:\n${stderr}`);
      const observation = requests.find(r => r.url === '/sessions/session-secret/observations');
      assert.ok(observation, 'expected observation request');
      const serialized = JSON.stringify(observation.body);
      assert.equal(serialized.includes('Bearer abcdefghijk'), false);
      assert.equal(serialized.includes('sk-proj-abcdefghi'), false);
      assert.equal(serialized.includes('ghp_abcdefghijk'), false);
      assert.match(serialized, /Bearer \[REDACTED\]/);
      assert.match(serialized, /sk-\[REDACTED\]/);
      assert.match(serialized, /ghp_\[REDACTED\]/);
    } finally {
      server.close();
    }
  });
});

describe('PreToolUse hook', () => {
  it('warns when a Read target is newer than related memory hints', async () => {
    const projectDir = await mkdtemp(join(tmpdir(), 'memfuse-mtime-'));
    const filePath = join(projectDir, 'src', 'auth.rs');
    await mkdir(join(projectDir, 'src'), { recursive: true });
    await writeFile(filePath, 'pub fn auth() {}\n', 'utf-8');

    const server = http.createServer((req, res) => {
      if (req.method === 'POST' && req.url === '/v1/memory:search') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          results: [{
            episode_id: 'ep-old-auth',
            summary: 'Old auth.rs memory about HS256',
            created_at: '2020-01-01T00:00:00Z',
          }],
        }));
        return;
      }
      if (req.method === 'GET' && req.url?.startsWith('/facts')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ facts: [] }));
        return;
      }
      if (req.method === 'POST' && req.url === '/heuristics/simulate-reaction') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ relevant_rules: [] }));
        return;
      }
      res.writeHead(404, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const child = spawn(
      process.execPath,
      ['dist/hooks/pre-tool-use.js'],
      {
        cwd: new URL('..', import.meta.url),
        env: {
          ...process.env,
          MEMFUSE_SERVER_URL: `http://127.0.0.1:${port}`,
          MEMFUSE_USER_ID: 'alice',
        },
        stdio: ['pipe', 'pipe', 'pipe'],
      },
    );

    let stdout = '';
    let stderr = '';
    child.stdout.on('data', chunk => { stdout += chunk; });
    child.stderr.on('data', chunk => { stderr += chunk; });
    child.stdin.end(JSON.stringify({
      hook_event_name: 'PreToolUse',
      session_id: 'session-read',
      tool_name: 'Read',
      tool_input: { file_path: filePath },
      user_id: 'alice',
    }));

    const exitCode = await new Promise(resolve => child.on('close', resolve));
    try {
      assert.equal(exitCode, 0, `stderr:\n${stderr}`);
      assert.match(stdout, /Old auth\.rs memory/);
      assert.match(stdout, /newer than related MemFuse memory/i);
    } finally {
      server.close();
    }
  });
});

// ─── 12. Session memory: learnings + pending extraction ───────────────────

describe('buildSessionMemory — Learnings extraction', () => {
  it('extracts lines containing learned/discovered/insight keywords', async () => {
    const { buildSessionMemory } = await import('../dist/hooks/stop.js');
    const message = [
      '# Session Memory',
      '',
      'This session we fixed the auth middleware ordering.',
      'I learned that Express middleware order matters for authentication guards.',
      'We discovered that the session store had a race condition under load.',
      'The insight about async/await patterns was useful for the refactor.',
      'Finally, all tests pass now.',
    ].join('\n');
    const result = buildSessionMemory(message);
    assert.ok(result, 'should produce session memory');
    assert.match(result, /Learnings/);
    assert.match(result, /learned that Express middleware/);
    assert.match(result, /discovered that the session store/);
    assert.match(result, /insight about async/);
  });

  it('caps learnings at 5 items', async () => {
    const { buildSessionMemory } = await import('../dist/hooks/stop.js');
    const lines = ['# Session Memory', '', 'Fixed the auth bug.'];
    for (let i = 0; i < 10; i++) {
      lines.push(`I discovered learning item ${i} about async patterns and concurrency.`);
    }
    const result = buildSessionMemory(lines.join('\n'));
    assert.ok(result);
    // Count items in the Learnings section specifically
    const learningsSection = result.split('## Learnings')[1] || '';
    const learningItems = learningsSection.match(/^- I discovered/gm) || [];
    assert.ok(learningItems.length <= 5, `expected at most 5 learnings, got ${learningItems.length}`);
    assert.ok(learningItems.length >= 1, 'should have at least 1 learning');
  });
});

describe('buildSessionMemory — Pending extraction', () => {
  it('extracts lines containing TODO/next step/待办 keywords', async () => {
    const { buildSessionMemory } = await import('../dist/hooks/stop.js');
    const message = [
      '# Session Memory',
      '',
      'We fixed the auth bug. All tests pass.',
      'TODO: add rate limiting to the API endpoints.',
      'Next step is to deploy to staging and run integration tests.',
      'Need to update the documentation for the new auth flow.',
      '待办: 添加错误处理中间件。',
    ].join('\n');
    const result = buildSessionMemory(message);
    assert.ok(result, 'should produce session memory');
    assert.match(result, /Pending/);
    assert.match(result, /TODO: add rate limiting/);
    assert.match(result, /Next step is to deploy/);
  });

  it('caps pending items at 5', async () => {
    const { buildSessionMemory } = await import('../dist/hooks/stop.js');
    const lines = ['# Session Memory', '', 'Completed the auth fix.'];
    for (let i = 0; i < 10; i++) {
      lines.push(`TODO: pending item ${i} needs to be completed before release.`);
    }
    const result = buildSessionMemory(lines.join('\n'));
    assert.ok(result);
    // Count items in the Pending section specifically
    const pendingSection = result.split('## Pending')[1] || '';
    const pendingItems = pendingSection.match(/^- TODO:/gm) || [];
    assert.ok(pendingItems.length <= 5, `expected at most 5 pending, got ${pendingItems.length}`);
    assert.ok(pendingItems.length >= 1, 'should have at least 1 pending item');
  });
});
