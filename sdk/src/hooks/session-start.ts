#!/usr/bin/env node
/**
 * MemFuse SessionStart Hook
 *
 * Injects relevant memory context at the beginning of each session.
 * Works on both Claude Code and Codex (platform detection via platform-utils).
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import { adaptInput, formatOutput, callBackend, readStdin, truncate, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, router } from './platform-utils.js';
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

    // 1. Resolve context from MemFuse
    let contextResult: Record<string, unknown>;
    try {
      const query = buildSessionStartQuery(input);
      contextResult = await callBackend('POST', PATHS.CONTEXT_RESOLVE, {
        user_id: userId, session_id: sessionId, query, token_budget: 1500,
      }, router) as Record<string, unknown>;
    } catch (err) {
      const level = isDegradableError(err) ? 'degraded' : 'error';
      process.stderr.write(`MemFuse SessionStart context ${level}: ${err instanceof Error ? err.message : String(err)}\n`);
      const healthWarning = '⚠️ MemFuse: Memory context unavailable. Manual tools still available via MCP.';
      process.stdout.write(formatOutput(platform, 'SessionStart', healthWarning));
      process.exit(EXIT_OK);
      return;
    }

    // 2. Format context for injection
    const formatted = formatContext(contextResult, platform);
    process.stdout.write(formatOutput(platform, 'SessionStart', formatted));
    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse SessionStart error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

function buildSessionStartQuery(input: Record<string, unknown>): string {
  const candidates = [
    input.prompt,
    input.user_prompt,
    input.initial_prompt,
    input.cwd ? `session continuity for ${String(input.cwd)}` : undefined,
  ];
  for (const candidate of candidates) {
    if (typeof candidate === 'string' && candidate.trim().length > 0) {
      return truncate(candidate.trim(), 240);
    }
  }
  return 'session continuity active facts recent work relevant context';
}

function formatContext(result: Record<string, unknown>, _platform: string): string {
  const sections = result.sections as Record<string, unknown> ?? {};
  const sectionsObj = typeof sections === 'object' && sections !== null ? sections as Record<string, unknown> : {};

  let output = '## MemFuse Memory Context\n';
  output += '> *Memory context from previous sessions. Use this to maintain continuity.*\n\n';

  // Active Facts
  const facts = toArray(sectionsObj.current_facts);
  if (facts.length > 0) {
    output += '### Active Facts\n';
    for (const fact of facts.slice(0, 20)) {
      const f = fact as Record<string, unknown>;
      const conf = Number(f.confidence || 0);
      const marker = conf >= 0.8 ? '✓' : '~';
      const subj = String(f.subject || '');
      const pred = String(f.predicate || '');
      const val = String(f.display_value || '');
      output += `- ${marker} **${subj}** → ${pred}: ${val}\n`;
    }
    output += '\n';
  }

  // Recent Updates
  const recent = toArray(sectionsObj.recent_updates);
  if (recent.length > 0) {
    output += '### Recent Activity\n';
    for (const update of recent.slice(0, 10)) {
      const u = update as Record<string, unknown>;
      const time = u.created_at ? new Date(String(u.created_at)).toLocaleTimeString('en-US', { hour: '2-digit', minute: '2-digit' }) : '';
      const action = String(u.tool_name || 'action');
      output += `- [${time}] ${action}: ${truncate(String(u.content || ''), 80)}\n`;
    }
    output += '\n';
  }

  // Relevant Episodes
  const episodes = toArray(sectionsObj.relevant_history);
  if (episodes.length > 0) {
    output += '### Relevant Episodes\n';
    for (const ep of episodes.slice(0, 5)) {
      const e = ep as Record<string, unknown>;
      const date = e.created_at ? new Date(String(e.created_at)).toLocaleDateString('en-US') : '';
      const relevance = String(e.relevance_score || '');
      output += `- **[${date}]** ${truncate(String(e.summary || ''), 100)} (relevance: ${relevance})\n`;
    }
    output += '\n';
  }

  // Behavioral Heuristics (T2H Phase 1)
  const heuristics = toArray(sectionsObj.behavioral_heuristics);
  if (heuristics.length > 0) {
    output += '### Behavioral Heuristics\n';
    output += '> *Learned preferences from past interactions — ★ confirmed, ◆ candidate, ○ draft*\n';
    for (const h of heuristics.slice(0, 5)) {
      const he = h as Record<string, unknown>;
      const stage = String(he.lifecycle_stage || 'draft');
      const marker = stage === 'confirmed' ? '★' : stage === 'candidate' ? '◆' : '○';
      const ruleText = String(he.rule_text || '');
      const counterExamples = toArray(he.counter_examples);
      output += `- ${marker} ${ruleText}`;
      if (counterExamples.length > 0) {
        output += ` ⚠️ except: ${counterExamples.map(String).join('; ')}`;
      }
      output += '\n';
    }
    output += '\n';
  }

  const hasContent = facts.length > 0 || recent.length > 0 || episodes.length > 0 || heuristics.length > 0;
  if (!hasContent) {
    output += '_No significant memory context found for this session._\n';
  }

  output += '\n_MemFuse MCP tools available: `search_memories`, `resolve_context`, `store_observation`, `list_facts`, `timeline`, `get_observations`, `memfuse_guide`_\n';

  return output;
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
