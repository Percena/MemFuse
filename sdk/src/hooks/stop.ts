#!/usr/bin/env node
/**
 * MemFuse Stop Hook
 *
 * Automatically stores a summary of the current turn when the Agent stops.
 * For Codex: also commits the session (since Codex lacks SessionEnd event).
 * Privacy: <private>...</private> blocks are stripped before storage.
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import { adaptInput, callBackend, readStdin, truncate, sanitizeMemoryText, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, ensureSession, router } from './platform-utils.js';

const config = loadConfig();

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    if (!rawInput.trim()) { process.exit(EXIT_OK); return; }

    const input = adaptInput(JSON.parse(rawInput));
    const userId = input.user_id || config.userId;
    const sessionId = input.session_id;
    const lastMessage = input.last_assistant_message;

    // Prevent infinite loop: if stop_hook_active, don't store again
    if (input.stop_hook_active) {
      process.exit(EXIT_OK);
      return;
    }

    // Skip empty messages
    if (!lastMessage || lastMessage.trim().length === 0) {
      process.exit(EXIT_OK);
      return;
    }

    // Strip private blocks and redact common token patterns before storage.
    const safeMessage = sanitizeMemoryText(lastMessage);
    const truncatedMessage = truncate(safeMessage, 3000);

    // Store the turn summary + structured session memory
    const observePath = `${PATHS.OBSERVE}/${sessionId || 'default'}/observations`;
    const turnContext = input.reason ? JSON.stringify({ reason: input.reason }) : '';
    await ensureSession(sessionId || 'default');

    // 1. Store turn summary (existing behavior)
    try {
      await callBackend('POST', observePath, {
        tool_name: 'TurnSummary',
        tool_input: turnContext,
        tool_output: truncatedMessage,
        content: `Turn summary: ${truncate(safeMessage, 200)}`,
        platform: input._platform,
      }, router);
    } catch (err) {
      if (isDegradableError(err)) {
        process.stderr.write(`MemFuse Stop write degraded: ${err instanceof Error ? err.message : String(err)}\n`);
      } else {
        process.stderr.write(`MemFuse Stop write error: ${err instanceof Error ? err.message : String(err)}\n`);
      }
    }

    // 2. Store structured session memory
    // Extract structured sections from the assistant's final message
    const sessionMemory = buildSessionMemory(safeMessage, input.reason ? String(input.reason) : undefined);
    if (sessionMemory) {
      try {
        await callBackend('POST', observePath, {
          tool_name: 'SessionMemory',
          tool_input: JSON.stringify({ session_id: sessionId, reason: input.reason }),
          tool_output: sessionMemory,
          content: 'Structured session memory snapshot',
          platform: input._platform,
        }, router);
      } catch (_) { /* non-critical */ }
    }

    // Codex lacks SessionEnd event, so commit on Stop to ensure memory consolidation.
    if (input._platform === 'codex' && sessionId) {
      try {
        const commitPath = `${PATHS.CONSOLIDATE}/${sessionId}/commit`;
        await callBackend('POST', commitPath, {
          user_id: userId,
          session_id: sessionId,
          reason: 'codex-stop-auto-commit',
        }, router);
      } catch (err) {
        // Commit failure is non-blocking — auto-commit threshold may have already committed,
        // or the server may be temporarily unavailable.
        if (isDegradableError(err)) {
          process.stderr.write(`MemFuse Stop commit degraded: ${err instanceof Error ? err.message : String(err)}\n`);
        } else {
          process.stderr.write(`MemFuse Stop commit error: ${err instanceof Error ? err.message : String(err)}\n`);
        }
      }
    }

    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse Stop error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

/**
 * Build a structured session memory from the assistant's final message.
 */
export function buildSessionMemory(message: string, reason?: string): string | null {
  if (!message || message.length < 100) return null;

  const lines = message.split('\n');
  const sections: Record<string, string[]> = {
    'Current State': [],
    'Key Results': [],
    'Files Modified': [],
    'Errors & Corrections': [],
    'Learnings': [],
    'Pending': [],
  };

  // Extract file paths mentioned in the message
  const filePathPattern = /(?:^|\s)((?:\/[\w.-]+)+\.\w+|[\w.-]+\/[\w.-]+(?:\/[\w.-]+)*\.\w+)/g;
  const files = new Set<string>();
  let match;
  while ((match = filePathPattern.exec(message)) !== null) {
    files.add(match[1]);
  }
  if (files.size > 0) {
    sections['Files Modified'] = [...files].slice(0, 20);
  }

  // Extract error-related lines
  for (const line of lines) {
    const lower = line.toLowerCase();
    if (lower.includes('error') || lower.includes('fix') || lower.includes('bug') || lower.includes('issue')) {
      if (line.trim().length > 10 && line.trim().length < 200) {
        sections['Errors & Corrections'].push(line.trim());
      }
    }
  }
  sections['Errors & Corrections'] = sections['Errors & Corrections'].slice(0, 5);

  // Extract learnings (bilingual keyword patterns)
  const learningPattern = /learned|discovered|insight|found that|realized|发现|学到|意识到|了解到/i;
  for (const line of lines) {
    const lower = line.toLowerCase();
    if (learningPattern.test(lower) && line.trim().length > 10 && line.trim().length < 200) {
      sections['Learnings'].push(line.trim());
    }
  }
  sections['Learnings'] = sections['Learnings'].slice(0, 5);

  // Extract pending items (bilingual keyword patterns)
  const pendingPattern = /TODO|next step|todo|待办|接下来|下一步|need to|should|记得|需要/i;
  for (const line of lines) {
    const lower = line.toLowerCase();
    if (pendingPattern.test(lower) && line.trim().length > 10 && line.trim().length < 200) {
      sections['Pending'].push(line.trim());
    }
  }
  sections['Pending'] = sections['Pending'].slice(0, 5);

  // Use the first meaningful paragraph as current state
  for (const line of lines) {
    const trimmed = line.trim();
    if (trimmed.length > 30 && !trimmed.startsWith('#') && !trimmed.startsWith('```')) {
      sections['Current State'].push(truncate(trimmed, 300));
      break;
    }
  }

  // Use the last meaningful paragraph as key results
  for (let i = lines.length - 1; i >= 0; i--) {
    const trimmed = lines[i].trim();
    if (trimmed.length > 30 && !trimmed.startsWith('#') && !trimmed.startsWith('```')) {
      sections['Key Results'].push(truncate(trimmed, 300));
      break;
    }
  }

  // Build markdown
  const parts: string[] = ['# Session Memory'];
  if (reason) parts.push(`\n**Reason**: ${reason}`);
  for (const [title, items] of Object.entries(sections)) {
    if (items.length === 0) continue;
    parts.push(`\n## ${title}`);
    for (const item of items) {
      parts.push(`- ${item}`);
    }
  }

  return parts.length > 2 ? parts.join('\n') : null;
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
