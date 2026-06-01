#!/usr/bin/env node
/**
 * MemFuse SessionEnd Hook
 *
 * Commits the current session to trigger memory extraction and consolidation.
 * Claude Code only — Codex does not have SessionEnd event.
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import { adaptInput, callBackend, readStdin, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, router } from './platform-utils.js';

const config = loadConfig();

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    if (!rawInput.trim()) { process.exit(EXIT_OK); return; }

    const input = adaptInput(JSON.parse(rawInput));
    const userId = input.user_id || config.userId;
    const sessionId = input.session_id;
    const reason = input.reason || 'unknown';

    // Commit session to trigger memory consolidation — POST /sessions/{sessionId}/commit
    const commitPath = `${PATHS.CONSOLIDATE}/${sessionId || 'default'}/commit`;
    try {
      await callBackend('POST', commitPath, {
        user_id: userId, session_id: sessionId, reason,
      }, router);
    } catch (err) {
      const level = isDegradableError(err) ? 'degraded' : 'error';
      process.stderr.write(`MemFuse SessionEnd commit ${level}: ${err instanceof Error ? err.message : String(err)}\n`);
    }

    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse SessionEnd error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
