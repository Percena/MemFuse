/**
 * MemFuse SDK Configuration
 *
 * Centralized configuration for all SDK components (MCP server, hooks, client).
 * Supports multi-endpoint routing (T0.6): cloudUrl + localCanvasUrl for SaaS,
 * but defaults to single endpoint via serverUrl for backward compatibility.
 */

import { existsSync, readFileSync } from 'node:fs';
import { homedir } from 'node:os';
import { join } from 'node:path';

/**
 * Canonical default port for the MemFuse server.
 *
 * Single source of truth on the TypeScript side — every SDK component
 * (hooks, MCP server, CLI, installer, service manager) must reference this
 * constant instead of hardcoding a port. At runtime the value is always
 * overridable via MEMFUSE_SERVER_URL (environment / .env) or
 * ~/.memfuse/config.toml. The Rust counterpart lives in mfs-types.
 */
export const DEFAULT_PORT = 18720;
export const DEFAULT_SERVER_URL = `http://127.0.0.1:${DEFAULT_PORT}`;
export const DEFAULT_BIND_ADDR = `127.0.0.1:${DEFAULT_PORT}`;

export interface MemFuseConfig {
  /** Primary server URL (backward compatible, used when cloudUrl/localCanvasUrl not set) */
  serverUrl: string;
  /** Cloud MemFuse SaaS API URL. Defaults to serverUrl for backward compatibility. */
  cloudUrl: string;
  /** Local Canvas Daemon URL. Defaults to serverUrl for backward compatibility. */
  localCanvasUrl: string;
  /** User ID for memory operations. Default: process.env.USER || 'default' */
  userId: string;
  /** Session ID for scoped operations. Default: '' (auto) */
  sessionId: string;
  /** Auth token for cloud API (optional, for SaaS mode) */
  authToken?: string;
  /** MemFuse internal data root when configured in the runtime config. */
  dataDir?: string;
}

interface FileConfig {
  client?: Record<string, string>;
  identity?: Record<string, string>;
  storage?: Record<string, string>;
}

/** Load configuration from environment variables and MemFuse user config */
export function loadConfig(configPath?: string): MemFuseConfig {
  const fileConfig = loadFileConfig(configPath);
  const serverUrl = process.env.MEMFUSE_SERVER_URL
    || fileConfig.client?.server_url
    || DEFAULT_SERVER_URL;
  return {
    serverUrl,
    cloudUrl: process.env.MEMFUSE_CLOUD_URL || fileConfig.client?.cloud_url || serverUrl,
    localCanvasUrl: process.env.MEMFUSE_LOCAL_CANVAS_URL
      || fileConfig.client?.local_canvas_url
      || serverUrl,
    userId: process.env.MEMFUSE_USER_ID
      || fileConfig.identity?.user_id
      || process.env.USER
      || 'default',
    sessionId: process.env.MEMFUSE_SESSION_ID || process.env.MEMFUSE_THREAD_ID || '',
    authToken: process.env.MEMFUSE_AUTH_TOKEN || fileConfig.client?.auth_token || undefined,
    dataDir: process.env.MEMFUSE_WORKSPACE_ROOT || fileConfig.storage?.data_dir || undefined,
  };
}

function loadFileConfig(configPath?: string): FileConfig {
  const resolvedPath = configPath || process.env.MEMFUSE_CONFIG || defaultConfigPath();
  if (!resolvedPath || !existsSync(resolvedPath)) return {};
  return parseMemFuseToml(readFileSync(resolvedPath, 'utf-8'));
}

export function memfuseHome(): string {
  return process.env.MEMFUSE_HOME || join(homedir(), '.memfuse');
}

export function defaultConfigPath(): string {
  return join(memfuseHome(), 'config.toml');
}

function parseMemFuseToml(raw: string): FileConfig {
  const config: FileConfig = {};
  let section: keyof FileConfig | undefined;
  for (const line of raw.split(/\r?\n/)) {
    const trimmed = line.trim();
    if (!trimmed || trimmed.startsWith('#')) continue;
    const sectionMatch = trimmed.match(/^\[(client|identity|storage)\]$/);
    if (sectionMatch) {
      section = sectionMatch[1] as keyof FileConfig;
      config[section] ||= {};
      continue;
    }
    if (!section) continue;
    const kv = trimmed.match(/^([A-Za-z0-9_]+)\s*=\s*"(.*)"\s*$/);
    if (kv) {
      config[section]![kv[1]] = kv[2];
    }
  }
  return config;
}
