/**
 * MemFuse HTTP Client
 *
 * Simplified HTTP helper for MCP server, hooks, and CLI.
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import * as http from 'node:http';
import * as https from 'node:https';
import { loadConfig, MemFuseConfig } from './config.js';
import { CanvasRouter, isCanvasWritePath, latestCanvasSnapshotPath } from './router.js';

const config = loadConfig();

/** Error class for HTTP-level failures (network errors, timeouts) */
export class MemFuseNetworkError extends Error {
  readonly status: number;
  readonly isCanvas: boolean;

  constructor(message: string, status: number, isCanvas: boolean = false) {
    super(message);
    this.name = 'MemFuseNetworkError';
    this.status = status;
    this.isCanvas = isCanvas;
  }
}

/**
 * Make an HTTP request to the MemFuse server.
 * Supports both http and https URLs.
 */
export function httpRequest(
  baseUrl: string,
  method: string,
  path: string,
  body: unknown,
  explicitApiKey?: string,
  isCanvas?: boolean,
): Promise<{ statusCode: number; body: unknown }> {
  const parsedUrl = new URL(path, baseUrl);
  const isHttps = parsedUrl.protocol === 'https:';
  const httpModule = isHttps ? https : http;

  const apiKey = explicitApiKey || process.env['MEMFUSE_API_KEY'];
  const authHeaders: Record<string, string> = apiKey
    ? { Authorization: `Bearer ${apiKey}` }
    : {};

  return new Promise((resolve, reject) => {
    const options: https.RequestOptions = {
      hostname: parsedUrl.hostname,
      port: parsedUrl.port || (isHttps ? 443 : 80),
      path: parsedUrl.pathname + parsedUrl.search,
      method,
      headers: { 'Content-Type': 'application/json', ...authHeaders },
      timeout: 8000,
    };

    if (isHttps) {
      options.rejectUnauthorized = !process.env['MEMFUSE_TLS_INSECURE'];
    }

    const req = httpModule.request(options, (res: http.IncomingMessage) => {
      let data = '';
      res.on('data', (chunk: Buffer) => { data += chunk; });
      res.on('end', () => {
        const statusCode = res.statusCode ?? 0;
        try {
          resolve({ statusCode, body: JSON.parse(data) });
        } catch (_) {
          resolve({ statusCode, body: { raw: data } });
        }
      });
    });

    req.on('error', (err) => {
      // Network-level error → status 0 indicates unreachable
      reject(new MemFuseNetworkError(
        `Network error: ${err.message}`,
        0,
        isCanvas ?? false,
      ));
    });
    req.on('timeout', () => {
      req.destroy();
      reject(new MemFuseNetworkError('Request timeout', 0, isCanvas ?? false));
    });
    if (body) req.write(JSON.stringify(body));
    req.end();
  });
}

/**
 * Call the MemFuse server (single-backend routing).
 * Legacy single-endpoint function, still used for backward compatibility.
 * All requests go directly to MEMFUSE_SERVER_URL.
 */
export async function callServer(
  method: string,
  path: string,
  body: unknown,
): Promise<unknown> {
  const result = await httpRequest(config.serverUrl, method, path, body);
  if (result.statusCode >= 400) {
    throw new Error(`Server returned ${result.statusCode}`);
  }
  return result.body;
}

/**
 * Call the MemFuse server with an explicit URL.
 * Used by callBackend for multi-endpoint routing.
 */
export async function callServerWithUrl(
  method: string,
  baseUrl: string,
  path: string,
  body: unknown,
  explicitApiKey?: string,
  isCanvas?: boolean,
): Promise<unknown> {
  const result = await httpRequest(baseUrl, method, path, body, explicitApiKey, isCanvas);
  if (result.statusCode >= 400) {
    throw new Error(`Server returned ${result.statusCode}`);
  }
  return result.body;
}

/**
 * Call the appropriate backend based on operation type (B1).
 *
 * Routes Canvas operations to localCanvasUrl, everything else to cloudUrl.
 * Includes offline degradation:
 *   - Canvas Daemon offline → stale snapshot from cloud (read-only) or unavailable (writes)
 *   - Cloud API offline → Canvas still works locally, others return degraded response
 *
 * @param method HTTP method
 * @param path API path
 * @param body Request body
 * @param router Optional CanvasRouter instance; defaults to one from loadConfig()
 */
export async function callBackend(
  method: string,
  path: string,
  body: unknown,
  router?: CanvasRouter,
): Promise<unknown> {
  const r = router || new CanvasRouter(config);
  const { url, isCanvas } = r.resolveBackend(path);
  const apiKey = r.apiAuthToken || undefined;

  try {
    return await callServerWithUrl(method, url, path, body, apiKey, isCanvas);
  } catch (err) {
    // Only apply degradation for network-level errors (status 0 = unreachable)
    if (!(err instanceof MemFuseNetworkError) || err.status !== 0) {
      throw err;
    }

    if (isCanvas) {
      // Canvas Daemon offline — degrade based on operation type
      const isWrite = isCanvasWritePath(path);
      if (isWrite) {
        // Write/real-time operations MUST return unavailable — cannot fake a live Canvas
        return {
          status: 'unavailable',
          hint: 'Canvas Daemon offline. Real-time canvas write operations require the local daemon.',
          freshness: 'unavailable',
        };
      }

      // Read operations → try stale snapshot from cloud
      console.warn('[MemFuse] Canvas Daemon offline, requesting stale cloud snapshot');
      try {
        // Reconstruct the query as a snapshot request on the cloud URL
        const snapshotPath = latestCanvasSnapshotPath(path);
        const staleResult = await callServerWithUrl(method, r.cloudApiUrl, snapshotPath, body, apiKey, true);
        // Mark the response as stale so the caller knows this is not live data
        if (typeof staleResult === 'object' && staleResult !== null) {
          return {
            ...staleResult as Record<string, unknown>,
            freshness: 'stale',
            hint: 'Canvas Daemon offline; data from last-synced cloud snapshot',
          };
        }
        return staleResult;
      } catch (cloudErr) {
        // Cloud also unavailable → full degradation
        return {
          status: 'unavailable',
          hint: 'Both Canvas Daemon and Cloud API are offline.',
          freshness: 'unavailable',
        };
      }
    }

    // Cloud API offline → Canvas still works, others degraded
    console.warn('[MemFuse] Cloud API offline, returning offline response');
    return {
      status: 'unavailable',
      hint: 'Cloud service unavailable. Canvas data is still available locally.',
      freshness: 'offline',
    };
  }
}

/**
 * Health check: verify MemFuse server is reachable and healthy.
 * Checks both network connectivity and HTTP status code.
 */
export async function checkHealth(): Promise<boolean> {
  try {
    const result = await httpRequest(config.serverUrl, 'GET', '/health', null);
    return result.statusCode >= 200 && result.statusCode < 400;
  } catch (_) {
    return false;
  }
}
