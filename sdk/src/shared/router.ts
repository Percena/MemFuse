/**
 * MemFuse Canvas Router — T0.6 Multi-Endpoint Configuration
 *
 * Routes SDK calls to the appropriate backend based on operation type.
 * In default mode (all URLs point to same server), behavior is identical
 * to current single-endpoint routing.
 */

import { MemFuseConfig } from './config.js';

/** Paths that should be routed to the local Canvas Daemon. */
const CANVAS_PATHS = [
  '/canvas/query',
  '/canvas/refresh',
  '/canvas/snapshot',
  '/canvas/version-hash',
  '/canvas/sync-status',
  '/v1/canvas/query',
  '/v1/canvas/refresh',
  '/v1/canvas/snapshot',
  '/v1/canvas/version-hash',
  '/v1/canvas/sync-status',
];

/** Canvas write/real-time paths that cannot fall back to stale snapshots. */
const CANVAS_WRITE_PATHS = [
  '/canvas/refresh',
  '/canvas/version-hash',
  '/canvas/snapshot',
  '/v1/canvas/refresh',
  '/v1/canvas/version-hash',
  '/v1/canvas/snapshot',
];

export interface BackendResolution {
  url: string;
  isCanvas: boolean;
}

function pathWithoutQuery(path: string): string {
  const queryStart = path.indexOf('?');
  return queryStart === -1 ? path : path.slice(0, queryStart);
}

function matchesRoute(path: string, route: string, includeChildren: boolean): boolean {
  const cleanPath = pathWithoutQuery(path);
  return cleanPath === route || (includeChildren && cleanPath.startsWith(`${route}/`));
}

export function isCanvasPath(path: string): boolean {
  return CANVAS_PATHS.some(cp => matchesRoute(path, cp, true));
}

export function isCanvasWritePath(path: string): boolean {
  return CANVAS_WRITE_PATHS.some(wp => matchesRoute(path, wp, false));
}

export function latestCanvasSnapshotPath(path: string): string {
  const query = path.includes('?') ? path.slice(path.indexOf('?')) : '';
  return path.startsWith('/v1/') ? `/v1/canvas/snapshot/latest${query}` : `/canvas/snapshot/latest${query}`;
}

/**
 * Routes SDK calls to the appropriate backend based on operation type.
 * In default mode (all URLs point to same server), behavior is identical
 * to current single-endpoint routing.
 */
export class CanvasRouter {
  private cloudUrl: string;
  private localCanvasUrl: string;
  private authToken?: string;

  constructor(config: MemFuseConfig) {
    this.cloudUrl = config.cloudUrl;
    this.localCanvasUrl = config.localCanvasUrl;
    this.authToken = config.authToken;
  }

  /** Get the backend URL for Canvas operations */
  get canvasUrl(): string {
    return this.localCanvasUrl;
  }

  /** Get the backend URL for non-Canvas (cloud) operations */
  get cloudApiUrl(): string {
    return this.cloudUrl;
  }

  /** Get the auth token for cloud API requests (if configured) */
  get apiAuthToken(): string | undefined {
    return this.authToken;
  }

  /** Determine which backend to use for a given API path */
  resolveBackend(path: string): BackendResolution {
    const isCanvas = isCanvasPath(path);
    return {
      url: isCanvas ? this.localCanvasUrl : this.cloudUrl,
      isCanvas,
    };
  }
}
