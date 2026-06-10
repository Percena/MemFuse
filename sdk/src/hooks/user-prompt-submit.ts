#!/usr/bin/env node
/**
 * MemFuse UserPromptSubmit Hook
 *
 * Injects a lightweight memory signal on each user prompt during a session.
 * This supplements the SessionStart injection (1500 tokens, once) with
 * per-prompt context that evolves as the conversation progresses.
 * Budget: 500 tokens — only the most relevant facts + recent signals.
 *
 * Claude Code only — Codex does not have a UserPromptSubmit event.
 */

import { adaptInput, formatOutput, callBackend, readStdin, truncate, sanitizeMemoryText, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, router } from './platform-utils.js';
import { toArray } from '../shared/utils.js';

const config = loadConfig();

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    if (!rawInput.trim()) { process.exit(EXIT_OK); return; }

    const input = adaptInput(JSON.parse(rawInput));
    const platform = input._platform;
    const userId = input.user_id || config.userId;
    const sessionId = input.session_id;

    // Extract the user's prompt text for semantic matching
    const promptText = sanitizeMemoryText(String(input.last_assistant_message || input.user_message || input.prompt || ''));

    // Skip if no meaningful prompt text (empty or very short)
    if (!promptText || promptText.trim().length < 5) {
      process.exit(EXIT_OK);
      return;
    }

    // Resolve lightweight context — budget 500 for mid-session signals
    let contextResult: Record<string, unknown>;
    try {
      contextResult = await callBackend('POST', PATHS.CONTEXT_RESOLVE, {
        user_id: userId,
        session_id: sessionId,
        query: truncate(promptText, 200),
        token_budget: 500,
        recall_source: 'auto',
      }, router) as Record<string, unknown>;
    } catch (err) {
      // Silent failure — mid-session signals are supplementary, not critical
      if (isDegradableError(err)) {
        process.stderr.write(`MemFuse UserPromptSubmit degraded: ${err instanceof Error ? err.message : String(err)}\n`);
      }
      process.exit(EXIT_OK);
      return;
    }

    // Format lightweight signal — only top facts + strongest episode signal
    const signal = formatSignal(contextResult);
    if (!signal) {
      process.exit(EXIT_OK);
      return;
    }

    process.stdout.write(formatOutput(platform, 'UserPromptSubmit', signal));
    process.exit(EXIT_OK);
  } catch (err) {
    // Never fail the prompt submission — this is supplementary only
    process.stderr.write(`MemFuse UserPromptSubmit error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

function formatSignal(result: Record<string, unknown>): string {
  const sections = result.sections as Record<string, unknown> ?? {};
  const sectionsObj = typeof sections === 'object' && sections !== null ? sections as Record<string, unknown> : {};

  // Top 3-5 facts with high confidence
  const facts = toArray(sectionsObj.current_facts)
    .filter((f) => Number((f as Record<string, unknown>).confidence || 0) >= 0.7)
    .slice(0, 5);

  // Top 1-2 episode signals
  const episodes = toArray(sectionsObj.relevant_history).slice(0, 2);

  // Top 1-2 confirmed/candidate heuristics relevant to the prompt
  const heuristics = toArray(sectionsObj.behavioral_heuristics)
    .filter((h) => {
      const stage = String((h as Record<string, unknown>).lifecycle_stage || '');
      return stage === 'confirmed' || stage === 'candidate';
    })
    .slice(0, 2);

  if (facts.length === 0 && episodes.length === 0 && heuristics.length === 0) {
    return '';  // No signal to inject — stay silent
  }

  let output = '📍 **MemFuse signal** (mid-session context update):\n';

  if (facts.length > 0) {
    for (const fact of facts) {
      const f = fact as Record<string, unknown>;
      const subj = String(f.subject || '');
      const pred = String(f.predicate || '');
      const val = String(f.display_value || '');
      output += `- ✓ **${subj}** → ${pred}: ${val}\n`;
    }
  }

  if (episodes.length > 0) {
    output += '\nRelevant past context:\n';
    for (const ep of episodes) {
      const e = ep as Record<string, unknown>;
      output += `- ${truncate(String(e.summary || ''), 80)}\n`;
    }
  }

  if (heuristics.length > 0) {
    output += '\nLearned preferences:\n';
    for (const h of heuristics) {
      const he = h as Record<string, unknown>;
      const stage = String(he.lifecycle_stage || 'draft');
      const marker = stage === 'confirmed' ? '★' : '◆';
      output += `- ${marker} ${String(he.rule_text || '')}\n`;
    }
  }

  output += '\n_(Use `resolve_context` or `search_memories` for deeper lookup)_\n';

  return output;
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
