/**
 * MemFuse Hook Platform Utilities
 *
 * Shared module for detecting platform, adapting input/output formats,
 * error classification, privacy control, and HTTP helpers.
 * Supports Claude Code and Codex.
 *
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import { callBackend, checkHealth } from '../shared/http.js';
import { CanvasRouter } from '../shared/router.js';
import { loadConfig } from '../shared/config.js';
import { stripPrivate, sanitizeSecrets, sanitizeMemoryText } from '../shared/privacy.js';
import { pathToFileURL } from 'node:url';

const config = loadConfig();
export const router = new CanvasRouter(config);

// ─── Exit Codes ────────────────────────────────────────────────────────
// exit 0: success or graceful degradation (non-blocking)
// exit 2: fatal error that should be reported to the host

export const EXIT_OK = 0;
export const EXIT_FATAL = 2;

// ─── Error Classification ──────────────────────────────────────────────
// Degradable errors: service unavailable, timeout, 5xx — agent can proceed
// Fatal errors: 4xx client errors, TypeError — indicate a bug

export function isDegradableError(err: unknown): boolean {
  if (!err) return false;
  // MemFuseNetworkError with status 0 = backend unreachable → degradable.
  if (err instanceof Error && err.name === 'MemFuseNetworkError') {
    const status = (err as Error & { status?: number }).status;
    if (status === 0) return true;
  }
  const msg = err instanceof Error ? err.message : '';
  if (msg.includes('ECONNREFUSED') || msg.includes('ECONNRESET') ||
      msg.includes('ETIMEDOUT') || msg.includes('ENOTFOUND') ||
      msg.includes('Request timeout') || msg.includes('fetch failed')) {
    return true;
  }
  if (/Server returned [5]\d\d/.test(msg)) return true;
  if (msg.includes('unavailable') || msg.includes('offline')) return true;
  return false;
}

// ─── Platform Detection ────────────────────────────────────────────────

export type Platform = 'claude-code' | 'codex';

/**
 * Explicit platform from `--platform=<p>` argv (written by the installer
 * into every hook command) or MEMFUSE_PLATFORM env. This is authoritative;
 * payload-shape heuristics (`detectPlatform`) are only a fallback — e.g.
 * Claude Code's Bash tool_input is `{command}` and would otherwise be
 * misclassified as Codex.
 */
export function platformFromArgv(): Platform | undefined {
  for (const arg of process.argv.slice(2)) {
    if (arg === '--platform=claude-code') return 'claude-code';
    if (arg === '--platform=codex') return 'codex';
  }
  const env = process.env.MEMFUSE_PLATFORM;
  if (env === 'claude-code' || env === 'codex') return env;
  return undefined;
}

export function detectPlatform(input: Record<string, unknown>): Platform {
  const explicit = platformFromArgv();
  if (explicit) return explicit;
  if (input.tool_input && typeof input.tool_input === 'object' && (input.tool_input as Record<string, unknown>).command) {
    return 'codex';
  }
  if (input.turn_id && !input.user_id && ('tool_response' in input || 'tool_name' in input)) {
    return 'codex';
  }
  if (input.hook_event_name === 'SessionStart' && input.source && typeof input.source === 'string') {
    // Platform hooks still send `thread_id` in payloads — read it as alias for `session_id`
    if (!input.thread_id && !input.session_id && input.transcript_path) {
      return 'codex';
    }
  }
  return 'claude-code';
}

// ─── Input Field Adaptation ────────────────────────────────────────────

export interface AdaptedInput extends Record<string, unknown> {
  _platform: Platform;
  session_id: string;
  user_id: string;
  cwd: string;
  tool_name: string;
  tool_input: string;
  tool_output: string;
  prompt: string;
  last_assistant_message: string;
}

export function adaptInput(rawInput: Record<string, unknown>): AdaptedInput {
  const platform = detectPlatform(rawInput);
  const input: AdaptedInput = { ...rawInput as Partial<AdaptedInput>, _platform: platform } as AdaptedInput;
  const sessionId = String(process.env.MEMFUSE_SESSION_ID || process.env.MEMFUSE_THREAD_ID || rawInput.session_id || rawInput.thread_id || '');

  if (platform === 'codex') {
    input.session_id = sessionId;
    input.user_id = config.userId;
    input.cwd = String(rawInput.cwd || process.cwd());

    if (rawInput.tool_name) {
      input.tool_name = String(rawInput.tool_name);
      if (rawInput.tool_input && typeof rawInput.tool_input === 'object' && (rawInput.tool_input as Record<string, unknown>).command) {
        input.tool_input = String((rawInput.tool_input as Record<string, unknown>).command);
      } else {
        input.tool_input = JSON.stringify(rawInput.tool_input || '');
      }
      input.tool_output = typeof rawInput.tool_response === 'string'
        ? String(rawInput.tool_response)
        : JSON.stringify(rawInput.tool_response || '');
    }

    if (rawInput.prompt) input.prompt = String(rawInput.prompt);
    if (rawInput.last_assistant_message) input.last_assistant_message = String(rawInput.last_assistant_message);
  } else {
    input.session_id = sessionId;
    input.user_id = String(rawInput.user_id || config.userId);
    input.cwd = String(rawInput.cwd || process.cwd());
    input.tool_name = String(rawInput.tool_name || '');
    input.tool_input = typeof rawInput.tool_input === 'object'
      ? JSON.stringify(rawInput.tool_input || '')
      : String(rawInput.tool_input || '');
    input.tool_output = String(rawInput.tool_output || rawInput.tool_response || '');
    input.prompt = String(rawInput.prompt || '');
    // Claude Code's Stop event delivers the final response as `assistant_message`.
    input.last_assistant_message = String(
      rawInput.last_assistant_message || rawInput.assistant_message || '',
    );
  }

  return input;
}

// ─── Privacy Control ───────────────────────────────────────────────────
export { stripPrivate, sanitizeSecrets, sanitizeMemoryText };

// ─── Output Format Adaptation ──────────────────────────────────────────

export function formatOutput(platform: Platform, eventName: string, content: string): string {
  if (platform === 'codex') {
    return JSON.stringify({
      hookSpecificOutput: {
        hookEventName: eventName,
        additionalContext: content,
      },
    });
  }
  // Claude Code: PreToolUse plain stdout is NOT injected into model context
  // (only SessionStart / UserPromptSubmit / PostToolUse support plain-text
  // injection). Use JSON additionalContext instead. Deliberately no
  // `permissionDecision` — memory hints must never bypass the user's
  // permission flow for the underlying tool call.
  if (eventName === 'PreToolUse') {
    return JSON.stringify({
      hookSpecificOutput: {
        hookEventName: 'PreToolUse',
        additionalContext: content,
      },
    });
  }
  return content;
}

// ─── API Paths (aligned with Rust Server routes) ────────────────────────
// No /mcp prefix — all paths match the mfs-server Axum routes directly.
// OBSERVE path requires session_id in URL: `/sessions/{sessionId}/observations`
// CONSOLIDATE requires session_id in URL: `/sessions/{sessionId}/commit`

export const PATHS = {
  OBSERVE: '/sessions',              // base — append `/{sessionId}/observations` at call site
  CONSOLIDATE: '/sessions',          // base — append `/{sessionId}/commit` at call site
  CONTEXT_RESOLVE: '/context/resolve',
  FACTS: '/facts',
  SEARCH: '/v1/memory:search',
  EPISODES: '/episodes',
  HEALTH: '/health',
  HEURISTICS_SIMULATE: '/heuristics/simulate-reaction',
  HEURISTICS_L0: '/heuristics/l0-confirmed',
} as const;

export async function ensureSession(sessionId: string): Promise<void> {
  if (!sessionId) return;
  try {
    await callBackend('POST', PATHS.OBSERVE, { session_id: sessionId }, router);
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (!/Server returned 409/.test(message)) throw err;
  }
}

// ─── Stdin Reader ──────────────────────────────────────────────────────

export function readStdin(): Promise<string> {
  return new Promise((resolve, reject) => {
    let data = '';
    process.stdin.on('data', (chunk: Buffer | string) => data += chunk);
    process.stdin.on('end', () => resolve(data));
    process.stdin.on('error', reject);
  });
}

export function isCliEntryPoint(metaUrl: string): boolean {
  const entry = process.argv[1];
  return Boolean(entry && pathToFileURL(entry).href === metaUrl);
}

// ─── String Utilities ──────────────────────────────────────────────────

export function truncate(str: string, maxLength: number): string {
  if (!str || str.length <= maxLength) return str || '';
  return str.substring(0, maxLength) + '\n... [truncated]';
}

// ─── Re-export shared modules ──────────────────────────────────────────

export { callBackend, checkHealth } from '../shared/http.js';
export { CanvasRouter } from '../shared/router.js';
export { loadConfig } from '../shared/config.js';
