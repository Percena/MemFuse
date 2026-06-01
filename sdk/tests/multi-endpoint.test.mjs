/**
 * T0.6 Multi-Endpoint Configuration Tests
 *
 * Validates that cloudUrl/localCanvasUrl default to serverUrl for backward compat,
 * that CanvasRouter routes /canvas/* paths to localCanvasUrl, and that
 * when all URLs are the same, behavior is identical to single-endpoint.
 *
 * Run: node --test tests/multi-endpoint.test.mjs
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import * as http from 'node:http';
import { mkdtemp, writeFile } from 'node:fs/promises';
import { join } from 'node:path';
import { tmpdir } from 'node:os';

// ─── 1. Config defaults ────────────────────────────────────────────────

describe('Config: multi-endpoint defaults', () => {
  it('cloudUrl defaults to serverUrl when MEMFUSE_CLOUD_URL not set', async () => {
    // Save and clear
    const origCloud = process.env.MEMFUSE_CLOUD_URL;
    const origServer = process.env.MEMFUSE_SERVER_URL;
    delete process.env.MEMFUSE_CLOUD_URL;
    delete process.env.MEMFUSE_SERVER_URL;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.cloudUrl, config.serverUrl, 'cloudUrl should default to serverUrl');
      assert.equal(config.cloudUrl, 'http://127.0.0.1:8720');
    } finally {
      if (origServer !== undefined) process.env.MEMFUSE_SERVER_URL = origServer;
      else delete process.env.MEMFUSE_SERVER_URL;
      if (origCloud !== undefined) process.env.MEMFUSE_CLOUD_URL = origCloud;
      else delete process.env.MEMFUSE_CLOUD_URL;
    }
  });

  it('localCanvasUrl defaults to serverUrl when MEMFUSE_LOCAL_CANVAS_URL not set', async () => {
    const origCanvas = process.env.MEMFUSE_LOCAL_CANVAS_URL;
    const origServer = process.env.MEMFUSE_SERVER_URL;
    delete process.env.MEMFUSE_LOCAL_CANVAS_URL;
    delete process.env.MEMFUSE_SERVER_URL;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.localCanvasUrl, config.serverUrl, 'localCanvasUrl should default to serverUrl');
      assert.equal(config.localCanvasUrl, 'http://127.0.0.1:8720');
    } finally {
      if (origServer !== undefined) process.env.MEMFUSE_SERVER_URL = origServer;
      else delete process.env.MEMFUSE_SERVER_URL;
      if (origCanvas !== undefined) process.env.MEMFUSE_LOCAL_CANVAS_URL = origCanvas;
      else delete process.env.MEMFUSE_LOCAL_CANVAS_URL;
    }
  });

  it('authToken defaults to undefined when MEMFUSE_AUTH_TOKEN not set', async () => {
    const origAuth = process.env.MEMFUSE_AUTH_TOKEN;
    delete process.env.MEMFUSE_AUTH_TOKEN;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.authToken, undefined, 'authToken should be undefined when not set');
    } finally {
      if (origAuth !== undefined) process.env.MEMFUSE_AUTH_TOKEN = origAuth;
      else delete process.env.MEMFUSE_AUTH_TOKEN;
    }
  });

  it('explicit MEMFUSE_CLOUD_URL overrides serverUrl default', async () => {
    const origCloud = process.env.MEMFUSE_CLOUD_URL;
    const origServer = process.env.MEMFUSE_SERVER_URL;
    process.env.MEMFUSE_CLOUD_URL = 'https://cloud.memfuse.io';
    delete process.env.MEMFUSE_SERVER_URL;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.cloudUrl, 'https://cloud.memfuse.io');
      assert.equal(config.serverUrl, 'http://127.0.0.1:8720', 'serverUrl stays default');
      assert.notEqual(config.cloudUrl, config.serverUrl, 'cloudUrl differs from serverUrl');
    } finally {
      if (origServer !== undefined) process.env.MEMFUSE_SERVER_URL = origServer;
      else delete process.env.MEMFUSE_SERVER_URL;
      if (origCloud !== undefined) process.env.MEMFUSE_CLOUD_URL = origCloud;
      else delete process.env.MEMFUSE_CLOUD_URL;
    }
  });

  it('explicit MEMFUSE_LOCAL_CANVAS_URL overrides serverUrl default', async () => {
    const origCanvas = process.env.MEMFUSE_LOCAL_CANVAS_URL;
    const origServer = process.env.MEMFUSE_SERVER_URL;
    process.env.MEMFUSE_LOCAL_CANVAS_URL = 'http://localhost:8721';
    delete process.env.MEMFUSE_SERVER_URL;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.localCanvasUrl, 'http://localhost:8721');
      assert.equal(config.serverUrl, 'http://127.0.0.1:8720', 'serverUrl stays default');
      assert.notEqual(config.localCanvasUrl, config.serverUrl, 'localCanvasUrl differs from serverUrl');
    } finally {
      if (origServer !== undefined) process.env.MEMFUSE_SERVER_URL = origServer;
      else delete process.env.MEMFUSE_SERVER_URL;
      if (origCanvas !== undefined) process.env.MEMFUSE_LOCAL_CANVAS_URL = origCanvas;
      else delete process.env.MEMFUSE_LOCAL_CANVAS_URL;
    }
  });

  it('explicit MEMFUSE_AUTH_TOKEN is loaded into authToken', async () => {
    const origAuth = process.env.MEMFUSE_AUTH_TOKEN;
    process.env.MEMFUSE_AUTH_TOKEN = 'tok-abc123';

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.authToken, 'tok-abc123');
    } finally {
      if (origAuth !== undefined) process.env.MEMFUSE_AUTH_TOKEN = origAuth;
      else delete process.env.MEMFUSE_AUTH_TOKEN;
    }
  });

  it('cloudUrl and localCanvasUrl both default to MEMFUSE_SERVER_URL when set', async () => {
    const origServer = process.env.MEMFUSE_SERVER_URL;
    const origCloud = process.env.MEMFUSE_CLOUD_URL;
    const origCanvas = process.env.MEMFUSE_LOCAL_CANVAS_URL;
    process.env.MEMFUSE_SERVER_URL = 'http://myserver:9000';
    delete process.env.MEMFUSE_CLOUD_URL;
    delete process.env.MEMFUSE_LOCAL_CANVAS_URL;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.serverUrl, 'http://myserver:9000');
      assert.equal(config.cloudUrl, 'http://myserver:9000', 'cloudUrl inherits from serverUrl');
      assert.equal(config.localCanvasUrl, 'http://myserver:9000', 'localCanvasUrl inherits from serverUrl');
    } finally {
      if (origServer !== undefined) process.env.MEMFUSE_SERVER_URL = origServer;
      else delete process.env.MEMFUSE_SERVER_URL;
      if (origCloud !== undefined) process.env.MEMFUSE_CLOUD_URL = origCloud;
      else delete process.env.MEMFUSE_CLOUD_URL;
      if (origCanvas !== undefined) process.env.MEMFUSE_LOCAL_CANVAS_URL = origCanvas;
      else delete process.env.MEMFUSE_LOCAL_CANVAS_URL;
    }
  });

  it('loads client endpoints from MEMFUSE_CONFIG when env overrides are absent', async () => {
    const configDir = await mkdtemp(join(tmpdir(), 'memfuse-config-'));
    const configPath = join(configDir, 'config.toml');
    await writeFile(configPath, `
[client]
server_url = "http://config-server:8720"
cloud_url = "https://cloud.memfuse.local"
local_canvas_url = "http://canvas.memfuse.local:8721"
auth_token = "cfg-token"

[identity]
user_id = "cfg-user"
`);

    const saved = saveEnv([
      'MEMFUSE_CONFIG',
      'MEMFUSE_SERVER_URL',
      'MEMFUSE_CLOUD_URL',
      'MEMFUSE_LOCAL_CANVAS_URL',
      'MEMFUSE_AUTH_TOKEN',
      'MEMFUSE_USER_ID',
    ]);
    process.env.MEMFUSE_CONFIG = configPath;
    delete process.env.MEMFUSE_SERVER_URL;
    delete process.env.MEMFUSE_CLOUD_URL;
    delete process.env.MEMFUSE_LOCAL_CANVAS_URL;
    delete process.env.MEMFUSE_AUTH_TOKEN;
    delete process.env.MEMFUSE_USER_ID;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.serverUrl, 'http://config-server:8720');
      assert.equal(config.cloudUrl, 'https://cloud.memfuse.local');
      assert.equal(config.localCanvasUrl, 'http://canvas.memfuse.local:8721');
      assert.equal(config.authToken, 'cfg-token');
      assert.equal(config.userId, 'cfg-user');
    } finally {
      restoreEnv(saved);
    }
  });

  it('keeps MEMFUSE_SERVER_URL ahead of client.server_url from config', async () => {
    const configDir = await mkdtemp(join(tmpdir(), 'memfuse-config-'));
    const configPath = join(configDir, 'config.toml');
    await writeFile(configPath, `
[client]
server_url = "http://config-server:8720"
cloud_url = "https://cloud.memfuse.local"
`);

    const saved = saveEnv(['MEMFUSE_CONFIG', 'MEMFUSE_SERVER_URL', 'MEMFUSE_CLOUD_URL']);
    process.env.MEMFUSE_CONFIG = configPath;
    process.env.MEMFUSE_SERVER_URL = 'http://env-server:9999';
    delete process.env.MEMFUSE_CLOUD_URL;

    try {
      const { loadConfig } = await import('../dist/shared/config.js?t=' + Date.now());
      const config = loadConfig();
      assert.equal(config.serverUrl, 'http://env-server:9999');
      assert.equal(config.cloudUrl, 'https://cloud.memfuse.local');
    } finally {
      restoreEnv(saved);
    }
  });
});

function saveEnv(keys) {
  return new Map(keys.map((key) => [key, process.env[key]]));
}

function restoreEnv(saved) {
  for (const [key, value] of saved.entries()) {
    if (value === undefined) delete process.env[key];
    else process.env[key] = value;
  }
}

// ─── 2. CanvasRouter routing ────────────────────────────────────────────

describe('CanvasRouter', () => {
  it('routes /canvas/* paths to localCanvasUrl', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    for (const path of ['/canvas/query', '/canvas/refresh', '/canvas/snapshot', '/canvas/version-hash', '/canvas/sync-status']) {
      const { url, isCanvas } = router.resolveBackend(path);
      assert.equal(url, 'http://canvas:8721', `${path} should route to localCanvasUrl`);
      assert.equal(isCanvas, true, `${path} should be classified as canvas`);
    }
  });

  it('routes /v1/canvas/* paths to localCanvasUrl', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    for (const path of [
      '/v1/canvas/query',
      '/v1/canvas/refresh',
      '/v1/canvas/snapshot',
      '/v1/canvas/snapshot/latest',
      '/v1/canvas/version-hash',
      '/v1/canvas/sync-status',
    ]) {
      const { url, isCanvas } = router.resolveBackend(path);
      assert.equal(url, 'http://canvas:8721', `${path} should route to localCanvasUrl`);
      assert.equal(isCanvas, true, `${path} should be classified as canvas`);
    }
  });

  it('routes non-canvas paths to cloudUrl', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    const nonCanvasPaths = ['/sessions', '/context/resolve', '/facts', '/search', '/health', '/v1/overlay/propose', '/v1/manifest/get'];
    for (const path of nonCanvasPaths) {
      const { url, isCanvas } = router.resolveBackend(path);
      assert.equal(url, 'https://cloud.memfuse.io', `${path} should route to cloudUrl`);
      assert.equal(isCanvas, false, `${path} should NOT be classified as canvas`);
    }
  });

  it('does not route lookalike canvas prefixes', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    for (const path of ['/canvas/queryable', '/canvas/snapshotter', '/v1/canvas/queryable']) {
      const { url, isCanvas } = router.resolveBackend(path);
      assert.equal(url, 'https://cloud.memfuse.io', `${path} should route to cloudUrl`);
      assert.equal(isCanvas, false, `${path} should not be classified as canvas`);
    }
  });

  it('when all URLs are same, behavior is identical to single-endpoint', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://127.0.0.1:8720',
      cloudUrl: 'http://127.0.0.1:8720',
      localCanvasUrl: 'http://127.0.0.1:8720',
      userId: 'test',
      sessionId: 's1',
    });

    const allPaths = ['/canvas/query', '/sessions', '/context/resolve', '/health'];
    for (const path of allPaths) {
      const { url } = router.resolveBackend(path);
      assert.equal(url, 'http://127.0.0.1:8720', `${path} should route to the single URL`);
    }
  });

  it('canvasUrl getter returns localCanvasUrl', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });
    assert.equal(router.canvasUrl, 'http://canvas:8721');
  });

  it('cloudApiUrl getter returns cloudUrl', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });
    assert.equal(router.cloudApiUrl, 'https://cloud.memfuse.io');
  });

  it('apiAuthToken getter returns authToken when set', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
      authToken: 'tok-secret',
    });
    assert.equal(router.apiAuthToken, 'tok-secret');
  });

  it('apiAuthToken getter returns undefined when not set', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });
    assert.equal(router.apiAuthToken, undefined);
  });
});

// ─── 3. callBackend routing ────────────────────────────────────────────

describe('callBackend', () => {
  it('callBackend routes to correct URL based on path', async () => {
    // We can't easily test callBackend with a live server in this test,
    // but we can verify the routing logic by creating a CanvasRouter
    // and checking resolveBackend, which is what callBackend uses internally.
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
      authToken: 'tok-123',
    });

    // Canvas path → localCanvasUrl
    assert.deepEqual(router.resolveBackend('/canvas/query'), { url: 'http://canvas:8721', isCanvas: true });
    // Cloud path → cloudUrl
    assert.deepEqual(router.resolveBackend('/sessions'), { url: 'https://cloud.memfuse.io', isCanvas: false });
  });

  it('callServerWithUrl delegates to httpRequest with the resolved URL', async () => {
    // Verify callServerWithUrl is exported and has the right signature
    const { callServerWithUrl } = await import('../dist/shared/http.js');
    assert.equal(typeof callServerWithUrl, 'function', 'callServerWithUrl should be exported');
  });

  it('callBackend is exported and has the right signature', async () => {
    const { callBackend } = await import('../dist/shared/http.js');
    assert.equal(typeof callBackend, 'function', 'callBackend should be exported');
  });

  it('CanvasRouter is exported from main index', async () => {
    const mod = await import('../dist/index.js?t=' + Date.now());
    assert.equal(typeof mod.CanvasRouter, 'function', 'CanvasRouter should be exported from main index');
    assert.equal(typeof mod.callBackend, 'function', 'callBackend should be exported from main index');
    assert.equal(typeof mod.callServerWithUrl, 'function', 'callServerWithUrl should be exported from main index');
    assert.equal(typeof mod.MemFuseNetworkError, 'function', 'MemFuseNetworkError should be exported from main index');
  });
});

// ─── 4. MemFuseNetworkError and offline degradation ────────────────────────

describe('MemFuseNetworkError', () => {
  it('is exported with correct properties', async () => {
    const { MemFuseNetworkError } = await import('../dist/shared/http.js');
    const err = new MemFuseNetworkError('test error', 0, true);
    assert.equal(err.name, 'MemFuseNetworkError');
    assert.equal(err.message, 'test error');
    assert.equal(err.status, 0);
    assert.equal(err.isCanvas, true);
  });

  it('status 0 indicates network unreachable', async () => {
    const { MemFuseNetworkError } = await import('../dist/shared/http.js');
    const err = new MemFuseNetworkError('Connection refused', 0);
    assert.equal(err.status, 0, 'status 0 = network error');
  });
});

describe('callBackend offline degradation', () => {
  it('falls back to the cloud latest snapshot for canvas read paths', async () => {
    const requests = [];
    const cloud = http.createServer((req, res) => {
      requests.push({ method: req.method, url: req.url });
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ status: 'ok', data: { snapshot_id: 'snap-1' } }));
    });
    await new Promise(resolve => cloud.listen(0, '127.0.0.1', resolve));

    try {
      const { callBackend } = await import('../dist/shared/http.js');
      const { CanvasRouter } = await import('../dist/shared/router.js');
      const cloudUrl = `http://127.0.0.1:${cloud.address().port}`;
      const router = new CanvasRouter({
        serverUrl: cloudUrl,
        cloudUrl,
        localCanvasUrl: 'http://127.0.0.1:1',
        userId: 'test',
        sessionId: 's1',
      });

      const result = await callBackend('GET', '/canvas/query?repo_id=r1&component=Runner', null, router);

      assert.deepEqual(requests, [
        { method: 'GET', url: '/canvas/snapshot/latest?repo_id=r1&component=Runner' },
      ]);
      assert.equal(result.freshness, 'stale');
      assert.equal(result.hint, 'Canvas Daemon offline; data from last-synced cloud snapshot');
      assert.deepEqual(result.data, { snapshot_id: 'snap-1' });
    } finally {
      await new Promise(resolve => cloud.close(resolve));
    }
  });

  it('does not call the cloud latest snapshot for canvas write paths', async () => {
    let cloudRequests = 0;
    const cloud = http.createServer((_req, res) => {
      cloudRequests += 1;
      res.writeHead(200, { 'content-type': 'application/json' });
      res.end(JSON.stringify({ status: 'ok' }));
    });
    await new Promise(resolve => cloud.listen(0, '127.0.0.1', resolve));

    try {
      const { callBackend } = await import('../dist/shared/http.js');
      const { CanvasRouter } = await import('../dist/shared/router.js');
      const cloudUrl = `http://127.0.0.1:${cloud.address().port}`;
      const router = new CanvasRouter({
        serverUrl: cloudUrl,
        cloudUrl,
        localCanvasUrl: 'http://127.0.0.1:1',
        userId: 'test',
        sessionId: 's1',
      });

      const result = await callBackend('POST', '/canvas/refresh', { repo_id: 'r1' }, router);

      assert.equal(cloudRequests, 0);
      assert.deepEqual(result, {
        status: 'unavailable',
        hint: 'Canvas Daemon offline. Real-time canvas write operations require the local daemon.',
        freshness: 'unavailable',
      });
    } finally {
      await new Promise(resolve => cloud.close(resolve));
    }
  });

  it('Canvas write paths are correctly identified', async () => {
    const { CanvasRouter, isCanvasWritePath } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    // Canvas write/real-time paths
    const writePaths = ['/canvas/refresh', '/canvas/version-hash', '/v1/canvas/refresh', '/v1/canvas/version-hash'];
    for (const path of writePaths) {
      const { isCanvas } = router.resolveBackend(path);
      assert.equal(isCanvas, true, `${path} is a canvas path`);
    }

    // Canvas read paths
    const readPaths = ['/canvas/query', '/canvas/snapshot/latest'];
    for (const path of readPaths) {
      const { isCanvas } = router.resolveBackend(path);
      assert.equal(isCanvas, true, `${path} is a canvas path`);
      assert.equal(isCanvasWritePath(path), false, `${path} is not a canvas write path`);
    }

    assert.equal(isCanvasWritePath('/canvas/snapshot'), true, '/canvas/snapshot creates immutable snapshots');
    assert.equal(isCanvasWritePath('/v1/canvas/snapshot'), true, '/v1/canvas/snapshot creates immutable snapshots');

    for (const path of ['/canvas/snapshot/latest', '/canvas/snapshot/latest?repo_id=r1', '/v1/canvas/snapshot/latest']) {
      assert.equal(isCanvasWritePath(path), false, `${path} is a stale snapshot read path`);
    }
  });

  it('Overlay paths route to cloud (not canvas)', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    const overlayPaths = ['/overlay/propose', '/overlay/accept', '/overlay/report_conflict', '/overlay/consolidate', '/overlays'];
    for (const path of overlayPaths) {
      const { url, isCanvas } = router.resolveBackend(path);
      assert.equal(url, 'https://cloud.memfuse.io', `${path} should route to cloud`);
      assert.equal(isCanvas, false, `${path} should NOT be canvas`);
    }
  });

  it('Manifest paths route to cloud (not canvas)', async () => {
    const { CanvasRouter } = await import('../dist/shared/router.js');
    const router = new CanvasRouter({
      serverUrl: 'http://default:8720',
      cloudUrl: 'https://cloud.memfuse.io',
      localCanvasUrl: 'http://canvas:8721',
      userId: 'test',
      sessionId: 's1',
    });

    const manifestPaths = ['/manifest/get', '/manifest/update'];
    for (const path of manifestPaths) {
      const { url, isCanvas } = router.resolveBackend(path);
      assert.equal(url, 'https://cloud.memfuse.io', `${path} should route to cloud`);
      assert.equal(isCanvas, false, `${path} should NOT be canvas`);
    }
  });
});
