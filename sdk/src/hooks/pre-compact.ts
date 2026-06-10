#!/usr/bin/env node
/**
 * MemFuse PreCompact Hook
 *
 * Saves the full current context before Claude Code compacts it.
 * Note: Codex does not have PreCompact event — this hook only runs on Claude Code.
 * Privacy: <private>...</private> blocks are stripped before storage.
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import { adaptInput, callBackend, readStdin, truncate, sanitizeMemoryText, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, router } from './platform-utils.js';

const config = loadConfig();

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    if (!rawInput.trim()) { process.exit(EXIT_OK); return; }

    const input = adaptInput(JSON.parse(rawInput));
    const userId = input.user_id || config.userId;
    const sessionId = input.session_id;

    // 1. Resolve current full context (before it gets compressed)
    let contextResult: unknown;
    try {
      contextResult = await callBackend('POST', PATHS.CONTEXT_RESOLVE, {
        user_id: userId,
        session_id: sessionId,
        query: 'pre-compact context snapshot current session memory',
        token_budget: 3000,
        recall_source: 'auto',
      }, router);
    } catch (err) {
      const level = isDegradableError(err) ? 'degraded' : 'error';
      process.stderr.write(`MemFuse PreCompact context resolve ${level}: ${err instanceof Error ? err.message : String(err)}\n`);
      process.exit(EXIT_OK);
    }

    // 2. Store the pre-compact context as an observation — POST /sessions/{sessionId}/observations
    const observePath = `${PATHS.OBSERVE}/${sessionId || 'default'}/observations`;
    const safeContext = sanitizeMemoryText(typeof contextResult === 'string'
      ? contextResult
      : JSON.stringify(contextResult));
    const truncated = truncate(safeContext, 10000);
    const trigger = input.trigger || 'unknown';

    try {
      await callBackend('POST', observePath, {
        tool_name: 'PreCompact',
        tool_input: JSON.stringify({ trigger }),
        tool_output: truncated,
        content: `Pre-compact context snapshot (${trigger} trigger)`,
        platform: input._platform || 'claude-code',
      }, router);
    } catch (err) {
      if (isDegradableError(err)) {
        process.stderr.write(`MemFuse PreCompact write degraded: ${err instanceof Error ? err.message : String(err)}\n`);
      } else {
        process.stderr.write(`MemFuse PreCompact write error: ${err instanceof Error ? err.message : String(err)}\n`);
      }
    }

    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse PreCompact error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
