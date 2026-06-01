#!/usr/bin/env node
/**
 * MemFuse PreToolUse[Read] Hook
 *
 * When the agent is about to read a file, inject relevant memory context
 * about that file. This implements the signal灯塔 principle: before the
 * agent digs into a resource, MemFuse shows WHERE relevant history lives.
 *
 * Only fires on Read tool calls (matcher: "Read").
 * Output is injected as additional context for the agent.
 */

import { adaptInput, formatOutput, callBackend, readStdin, truncate, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, router } from './platform-utils.js';
import { toArray } from '../shared/utils.js';
import { stat } from 'node:fs/promises';

const config = loadConfig();

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    if (!rawInput.trim()) { process.exit(EXIT_OK); return; }

    const input = adaptInput(JSON.parse(rawInput));
    const platform = input._platform;
    const userId = input.user_id || config.userId;
    const sessionId = input.session_id;
    const toolName = input.tool_name;

    // Only act on Read tool calls
    if (toolName !== 'Read') {
      process.exit(EXIT_OK);
      return;
    }

    // Extract file path from Read tool input
    const toolInput = input.tool_input;
    let filePath = '';
    try {
      const parsed = JSON.parse(toolInput);
      filePath = parsed.file_path || parsed.uri || '';
    } catch (_) {
      // Non-JSON input — try to extract path from raw string
      filePath = String(toolInput).replace(/^["']|["']$/g, '');
    }

    if (!filePath || filePath.length < 3) {
      process.exit(EXIT_OK);
      return;
    }

    // Search memories related to this file path
    let contextLines: string[] = [];
    try {
      const searchResult = await callBackend('POST', PATHS.SEARCH, {
        user_id: userId,
        session_id: sessionId,
        query: filePath,
        limit: 3,
      }, router) as Record<string, unknown>;

      const results = toArray(searchResult.results);
      if (results.length > 0) {
        contextLines.push(`📝 MemFuse: Past observations related to ${filePath}:`);
        for (const r of results) {
          const summary = truncate(String(r.summary || ''), 120);
          const id = String(r.episode_id || '');
          contextLines.push(`  - ${summary} 📍 episode=${id}`);
        }
        const latestMemoryMs = latestTimestampMs(results);
        const fileMtimeMs = await readableFileMtimeMs(filePath);
        if (latestMemoryMs !== null && fileMtimeMs !== null && fileMtimeMs > latestMemoryMs) {
          contextLines.push('  ⚠ Target file is newer than related MemFuse memory; verify hints against the current file contents.');
        }
        contextLines.push('Use `get_observations` or `timeline` for full detail.');
      }
    } catch (err) {
      if (isDegradableError(err)) {
        // Silent — don't block the Read operation
      } else {
        process.stderr.write(`MemFuse PreToolUse search error: ${err instanceof Error ? err.message : String(err)}\n`);
      }
    }

    // Also check active facts for file-related predicates
    try {
      const factsResult = await callBackend('GET', `${PATHS.FACTS}?user_id=${encodeURIComponent(userId)}`, null, router) as Record<string, unknown>;
      const facts = toArray(factsResult.facts);
      // Filter facts whose value or predicate mentions the file path
      const relatedFacts = facts.filter((f: Record<string, unknown>) => {
        const val = String(f.display_value || '');
        return val.includes(filePath) || filePath.includes(val);
      });
      if (relatedFacts.length > 0 && contextLines.length === 0) {
        contextLines.push(`📝 MemFuse: Known facts related to ${filePath}:`);
        for (const f of relatedFacts.slice(0, 3)) {
          const conf = Number(f.confidence || 0);
          const marker = conf >= 0.8 ? '✓' : '~';
          const val = truncate(String(f.display_value || ''), 100);
          const pred = String(f.predicate || '');
          contextLines.push(`  - ${marker} ${val} 📍 [${pred}]`);
        }
      }
    } catch (_) {
      // Silent — facts lookup failure doesn't block
    }

    // ── L2 Heuristic injection (roadmap §10.2) ──
    // Before reading/writing files, check if any learned preference rules apply
    try {
      const heuristicResult = await callBackend(
        'POST',
        `${PATHS.HEURISTICS_SIMULATE}`,
        { scenario: filePath, tags: [], user_id: userId },
        router,
      ) as Record<string, unknown>;
      const rules = toArray(heuristicResult.relevant_rules as unknown);
      if (rules.length > 0) {
        if (contextLines.length === 0) {
          contextLines.push(`💡 MemFuse: Learned preferences relevant to this file:`);
        } else {
          contextLines.push(`💡 Learned preferences:`);
        }
        for (const r of rules.slice(0, 3)) {
          const stage = String(r.lifecycle_stage || 'draft');
          const marker = stage === 'confirmed' ? '★' : stage === 'candidate' ? '◆' : '○';
          const text = truncate(String(r.rule_text || ''), 100);
          contextLines.push(`  ${marker} ${text}`);
        }
        if (String(heuristicResult.prediction || '').length > 0) {
          contextLines.push(`  → ${truncate(String(heuristicResult.prediction), 120)}`);
        }
      }
    } catch (_) {
      // Silent — heuristic lookup failure doesn't block
    }

    if (contextLines.length > 0) {
      const content = contextLines.join('\n');
      process.stdout.write(formatOutput(platform, 'PreToolUse', content));
    }

    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse PreToolUse error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

async function readableFileMtimeMs(filePath: string): Promise<number | null> {
  try {
    const info = await stat(filePath);
    return info.isFile() ? info.mtimeMs : null;
  } catch (_) {
    return null;
  }
}

function latestTimestampMs(records: Record<string, unknown>[]): number | null {
  let latest: number | null = null;
  for (const record of records) {
    for (const key of ['updated_at', 'last_recalled_at', 'created_at']) {
      const value = record[key];
      if (typeof value !== 'string' || value.length === 0) continue;
      const parsed = Date.parse(value);
      if (Number.isNaN(parsed)) continue;
      latest = latest === null ? parsed : Math.max(latest, parsed);
    }
  }
  return latest;
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
