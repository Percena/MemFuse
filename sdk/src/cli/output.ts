/**
 * MemFuse CLI — Token-efficient output formatting
 *
 * Three modes: default (compact), json (raw), verbose (full MCP-style)
 * Key differences from MCP output: no emoji markers, no footer tips,
 * no repeated ## headers, compact ID format.
 *
 * See docs/architecture.md §10 CLI Architecture §4 for format specification.
 */

import { extractErrorMessage, toArray } from '../shared/utils.js';

export type OutputMode = 'default' | 'json' | 'verbose';

// ─── Search Results ──────────────────────────────────────────────────

export function formatSearchResults(result: Record<string, unknown>, mode: OutputMode): string {
  const results = toArray(result.results);
  const total = Number(result.total || results.length);

  if (mode === 'json') return JSON.stringify(result);
  if (results.length === 0) return 'No memories found.';

  const lines = results.map((r: Record<string, unknown>) => {
    const id = String(r.episode_id || r.id || '');
    const summary = String(r.summary || r.content || '');
    const score = Number(r.score || 0).toFixed(2);
    const salience = Number(r.salience_score || 0).toFixed(2);
    const date = String(r.created_at || '');
    const dateStr = date ? new Date(date).toLocaleDateString() : '';
    if (mode === 'verbose') {
      return `- **[${dateStr}]** ${summary}\n  ID: \`${id}\` | relevance: ${score} | importance: ${salience}`;
    }
    return `${dateStr} ${id} ${summary} (r:${score} i:${salience})`;
  });

  if (mode === 'verbose') {
    return `## Search Results (${total} found)\n\n${lines.join('\n\n')}\n\n---\n*Use \`get_observations\` with the IDs above for more detail.*`;
  }
  return `Search (${total}):\n${lines.join('\n')}`;
}

// ─── Timeline ────────────────────────────────────────────────────────

export function formatTimeline(anchorId: string, episodes: Record<string, unknown>[], facts: Record<string, unknown>[], mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify({ anchor_id: anchorId, episodes, facts });

  const parts: string[] = [];

  if (facts.length > 0) {
    parts.push(mode === 'verbose' ? '### Related Facts' : 'Facts:');
    parts.push(facts.map((f: Record<string, unknown>) => {
      const subj = String(f.subject || '');
      const pred = String(f.predicate || '');
      const val = String(f.display_value || '');
      return mode === 'verbose' ? `- **${subj}** → ${pred}: ${val}` : `${subj} → ${pred}: ${val}`;
    }).join('\n'));
  }

  if (episodes.length > 0) {
    parts.push(mode === 'verbose' ? '### Episode Timeline' : 'Timeline:');
    parts.push(episodes.map((ep: Record<string, unknown>) => {
      const id = String(ep.episode_id || '');
      const summary = String(ep.summary || '');
      const score = Number(ep.score || 0).toFixed(2);
      const date = String(ep.created_at || '');
      const dateStr = date ? new Date(date).toLocaleDateString() : '';
      const marker = id === anchorId ? (mode === 'verbose' ? '▶ **[NOW]**' : '>>>') : `[${dateStr}]`;
      return mode === 'verbose'
        ? `${marker} ${summary} (relevance: ${score})\n  ID: \`${id}\``
        : `${marker} ${id} ${summary} (${score})`;
    }).join('\n'));
    if (mode === 'verbose') parts.push('---\n*Use `get_observations` with specific IDs for full details.*');
  }

  if (parts.length === 0) return 'No context found.';
  if (mode === 'verbose') return `## Timeline around \`${anchorId}\`\n\n${parts.join('\n')}`;
  return parts.join('\n');
}

// ─── Observations ────────────────────────────────────────────────────

export function formatObservations(details: Record<string, unknown>[], mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(details);
  if (details.length === 0) return 'No details retrieved.';

  const parts: string[] = [];
  details.forEach((d: Record<string, unknown>, i: number) => {
    if (d.error) { parts.push(`${String(d.id)}: ${extractErrorMessage(d.error)}`); return; }

    const id = String(d.episode_id || d.id || `detail-${i}`);
    const summary = String(d.summary || d.content || '');
    const facts = toArray(d.facts);
    const created = String(d.created_at || '');

    if (mode === 'verbose') {
      parts.push(`### (${i + 1}) Episode: \`${id}\``);
      if (created) parts.push(`Created: ${new Date(created).toLocaleString()}`);
      if (summary) parts.push(`**Summary:** ${summary}`);
      if (facts.length > 0) {
        parts.push('**Facts:**');
        facts.forEach((f: Record<string, unknown>) => {
          parts.push(`- ${String(f.subject || '')} → ${String(f.predicate || '')}: ${String(f.display_value || '')}`);
        });
      }
    } else {
      let line = `${id}: ${summary}`;
      if (created) line += ` (${new Date(created).toLocaleDateString()})`;
      parts.push(line);
      if (facts.length > 0) {
        parts.push(facts.map((f: Record<string, unknown>) =>
          `  ${String(f.subject || '')} → ${String(f.predicate || '')}: ${String(f.display_value || '')}`
        ).join('\n'));
      }
    }
  });

  if (mode === 'verbose') return `## Observation Details\n\n${parts.join('\n')}`;
  return parts.join('\n');
}

// ─── Resolve Context ─────────────────────────────────────────────────

export function formatResolveContext(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);

  const rendered = result.rendered_markdown || '';
  if (rendered && typeof rendered === 'string' && mode === 'verbose') return rendered;

  // Primary path: assemble from sections (produces spec-aligned compact format)
  const sections = (result.sections as Record<string, unknown>) ?? {};
  const facts = toArray(sections.current_facts);
  const history = toArray(sections.relevant_history);
  const recent = toArray(sections.recent_updates);
  const heuristics = toArray(sections.behavioral_heuristics as unknown);

  if (facts.length > 0 || history.length > 0 || recent.length > 0 || heuristics.length > 0) {
    const parts: string[] = [];

    if (facts.length > 0) {
      parts.push(mode === 'verbose' ? '## Active Facts' : 'Facts:');
      parts.push(facts.map((f: Record<string, unknown>) => {
        const conf = Number(f.confidence || 0);
        const subj = String(f.subject || '');
        const pred = String(f.predicate || '');
        const val = String(f.display_value || '');
        if (mode === 'verbose') return `- ${confStr(conf, 'verbose')} **${subj}** → ${pred}: ${val} (confidence: ${conf.toFixed(2)})`;
        return `${subj} → ${pred}: ${val} (${conf.toFixed(2)})`;
      }).join('\n'));
    }

    if (recent.length > 0) {
      parts.push(mode === 'verbose' ? '## Recent Updates' : 'Updates:');
      parts.push(recent.map((r: unknown) => `- ${r}`).join('\n'));
    }

    if (history.length > 0) {
      parts.push(mode === 'verbose' ? '## Relevant Episodes' : 'Episodes:');
      parts.push(history.map((ep: Record<string, unknown>) => {
        const id = String(ep.episode_id || '');
        const summary = String(ep.summary || '');
        const score = Number(ep.score || 0).toFixed(2);
        if (mode === 'verbose') return `- **[${id}]** ${summary} (score: ${score})`;
        return `${id} ${summary} (${score})`;
      }).join('\n'));
    }

    if (heuristics.length > 0) {
      parts.push(mode === 'verbose' ? '## Behavioral Heuristics' : 'Heuristics:');
      parts.push(heuristics.map((h: Record<string, unknown>) => {
        const stage = String(h.lifecycle_stage || 'draft');
        const text = String(h.rule_text || '');
        const tags = String(h.tags || '');
        if (mode === 'verbose') return `- ${stageStr(stage, 'verbose')} **${text}** [${tags}]`;
        return `${stageStr(stage, 'default')} ${text} [${tags}]`;
      }).join('\n'));
    }

    return parts.length > 0 ? parts.join('\n\n') : 'No relevant context found.';
  }

  // Fallback: strip verbose markers from rendered_markdown
  if (rendered && typeof rendered === 'string') {
    return stripVerboseMarkers(rendered);
  }

  return 'No relevant context found.';
}

function stripVerboseMarkers(md: string): string {
  return md
    .replace(/^## /gm, '')
    .replace(/^### /gm, '')
    .replace(/^---\n\*[^\n]+\*$/gm, '')
    .replace(/^---$/gm, '')
    .replace(/[✓~?★◆○📁📄▶]/g, '')
    .replace(/⚠\ufe0f?/g, '')
    .replace(/\*{1,2}(.*?)\*{1,2}/g, '$1')
    .replace(/^`[^`]+`$/gm, '')
    .trim();
}

// ─── Context Search (grep/find) ──────────────────────────────────────

export function formatContextSearchResults(title: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);

  const buckets = [
    ['resources', toArray(result.resources)],
    ['memories', toArray(result.memories)],
    ['skills', toArray(result.skills)],
  ] as const;

  const lines: string[] = [];
  for (const [label, items] of buckets) {
    if (items.length === 0) continue;
    lines.push(mode === 'verbose' ? `### ${capitalize(label)}` : `${capitalize(label)}:`);
    for (const item of items) {
      const uri = String((item as Record<string, unknown>).uri || '');
      const score = Number((item as Record<string, unknown>).score || 0).toFixed(2);
      const summary = String((item as Record<string, unknown>).summary || (item as Record<string, unknown>).content || '');
      if (mode === 'verbose') lines.push(`- \`${uri}\` (score: ${score})\n  ${summary}`);
      else lines.push(`${uri} (${score}) ${summary}`);
    }
  }

  if (lines.length === 0) return `${title}: No results found.`;
  if (mode === 'verbose') return `## ${title}\n\n${lines.join('\n')}`;
  return `${title}:\n${lines.join('\n')}`;
}

// ─── Listing ──────────────────────────────────────────────────────────

export function formatLs(uri: string, entries: { name: string; is_dir: boolean }[], mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify({ uri, entries });
  if (entries.length === 0) return `No entries for ${uri}.`;

  const baseUri = uri.endsWith('/') ? uri.slice(0, -1) : uri;
  if (mode === 'verbose') {
    return `## Listing for \`${uri}\`\n\n${entries.map(item => `- ${item.is_dir ? '📁' : '📄'} ${item.name}\n  \`${baseUri}/${item.name}\``).join('\n')}`;
  }
  return entries.map(item => `${item.is_dir ? 'd' : '-'} ${baseUri}/${item.name}`).join('\n');
}

// ─── Facts ────────────────────────────────────────────────────────────

export function formatFacts(facts: Record<string, unknown>[], mode: OutputMode, rawResult?: Record<string, unknown>): string {
  if (mode === 'json') return JSON.stringify(rawResult ?? { facts });
  if (facts.length === 0) return 'No active facts.';

  const lines = facts.map((f: Record<string, unknown>) => {
    const conf = Number(f.confidence || 0);
    const subj = String(f.subject || '');
    const pred = String(f.predicate || '');
    const val = String(f.display_value || '');
    if (mode === 'verbose') return `- ${confStr(conf, 'verbose')} **${subj}** → ${pred}: ${val} (confidence: ${conf.toFixed(2)})`;
    return `${subj} → ${pred}: ${val} (${conf.toFixed(2)})`;
  });

  if (mode === 'verbose') return `## Active Facts (${facts.length} total)\n\n${lines.join('\n')}`;
  return `Facts (${facts.length}):\n${lines.join('\n')}`;
}

// ─── Simple Text (abstract, overview, read) ───────────────────────────

export function formatText(label: string, uri: string, text: string, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify({ uri, label, content: text });
  if (mode === 'verbose') return `## ${label} for \`${uri}\`\n\n${text}`;
  if (!text) return `No ${label} for ${uri}.`;
  return text;
}

export function formatTextRaw(result: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const r = result as Record<string, unknown>;
  const text = typeof result === 'string' ? result : String(r?.raw ?? '');
  const uri = String(r?.uri ?? '');
  const label = String(r?.label ?? 'Content');
  if (mode === 'verbose') return `## ${label} for \`${uri}\`\n\n${text}`;
  if (!text) return 'No content.';
  return text;
}

// ─── Glob ──────────────────────────────────────────────────────────────

export function formatGlob(uri: string, pattern: string, matches: string[], mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify({ uri, pattern, matches });
  if (matches.length === 0) return `No matches for ${pattern} under ${uri}.`;
  if (mode === 'verbose') {
    return `## Glob Matches for \`${pattern}\` under \`${uri}\`\n\n${matches.map(m => `- ${m}`).join('\n')}`;
  }
  return matches.join('\n');
}

// ─── Session ──────────────────────────────────────────────────────────

export function formatSession(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.session_id || '');
  const status = String(result.status || '');
  const created = String(result.created_at || '');
  const turns = Number(result.turn_count || 0);
  if (mode === 'verbose') {
    const lines = [`## Session: \`${id}\``];
    lines.push(`Status: ${status}`);
    if (created) lines.push(`Created: ${new Date(created).toLocaleString()}`);
    if (turns) lines.push(`Turns: ${turns}`);
    return lines.join('\n');
  }
  return `${id} ${status}${turns ? ` turns:${turns}` : ''}`;
}

export function formatSessionList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).sessions || results
    : results;
  const sessions = toArray(payload);
  if (sessions.length === 0) return 'No sessions found.';
  if (mode === 'verbose') {
    return `## Sessions (${sessions.length})\n\n${sessions.map(s => `- \`${String(s.session_id || '')}\` ${String(s.status || '')}`).join('\n')}`;
  }
  return sessions.map(s => `${String(s.session_id || '')} ${String(s.status || '')}`).join('\n');
}

// ─── Observation Stored ───────────────────────────────────────────────

export function formatObservationStored(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const turnId = String(result.turn_id || 'stored');
  const autoCommitted = Boolean(result.auto_committed);
  if (mode === 'verbose') {
    const lines = ['## Observation Stored'];
    lines.push(`✓ Turn ID: \`${turnId}\``);
    if (autoCommitted) lines.push('★ Auto-committed: session threshold reached');
    return lines.join('\n');
  }
  return `Stored ${turnId}${autoCommitted ? ' (auto-committed)' : ''}`;
}

// ─── Session Committed ────────────────────────────────────────────────

export function formatSessionCommitted(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const archiveUri = String(result.archive_uri || '');
  const taskId = result.task_id ? String(result.task_id) : '';
  if (mode === 'verbose') {
    const lines = ['## Session Committed'];
    if (archiveUri) lines.push(`✓ Archive: \`${archiveUri}\``);
    if (taskId) lines.push(`Task: \`${taskId}\``);
    if (!archiveUri) lines.push('~ No new content to archive');
    return lines.join('\n');
  }
  if (!archiveUri) return 'Committed (no new content).';
  return `Committed → ${archiveUri}${taskId ? ` task:${taskId}` : ''}`;
}

// ─── Resource Added ───────────────────────────────────────────────────

export function formatResourceAdded(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const resourceId = String(result.resource_id || '');
  const taskKey = String(result.task_key || '');
  if (mode === 'verbose') {
    const lines = ['## Resource Added'];
    lines.push(`✓ Resource ID: \`${resourceId}\``);
    if (taskKey) lines.push(`Task: \`${taskKey}\``);
    return lines.join('\n');
  }
  return `Added ${resourceId}${taskKey ? ` task:${taskKey}` : ''}`;
}

export function formatResourceList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).resources || results
    : results;
  const items = toArray(payload);
  if (items.length === 0) return 'No resources registered.';
  if (mode === 'verbose') {
    return `## Resources (${items.length})\n\n${items.map(i => `- \`${String(i.resource_id || '')}\` ${String(i.logical_name || '')} (${String(i.status || '')})`).join('\n')}`;
  }
  return items.map(i => `${String(i.resource_id || '')} ${String(i.logical_name || '')} ${String(i.status || '')}`).join('\n');
}

// ─── Task ─────────────────────────────────────────────────────────────

export function formatTaskStatus(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const status = String(result.status || 'unknown');
  const processingMode = String(result.processing_mode || '');
  const error = result.error ? extractErrorMessage(result.error) : '';
  if (mode === 'verbose') {
    const lines = [`## Task Status: ${status}`];
    if (processingMode) lines.push(`Mode: ${processingMode}`);
    if (error) lines.push(`⚠ Error: ${error}`);
    return lines.join('\n');
  }
  if (status === 'completed') return `Done (${processingMode})`;
  if (status === 'failed') return `Failed: ${error}`;
  return `${status} (${processingMode})`;
}

export function formatTaskList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).tasks || results
    : results;
  const tasks = toArray(payload);
  const taskId = (task: Record<string, unknown>) => String(task.task_key || task.task_id || task.id || '');
  if (tasks.length === 0) return 'No tasks found.';
  if (mode === 'verbose') {
    return `## Tasks (${tasks.length})\n\n${tasks.map(t => `- \`${taskId(t)}\` ${String(t.status || '')}`).join('\n')}`;
  }
  return tasks.map(t => `${taskId(t)} ${String(t.status || '')}`).join('\n');
}

// ─── Resource Operations ──────────────────────────────────────────────

export function formatResourceOp(label: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const taskKey = String(result.task_key || '');
  if (mode === 'verbose') return `## ${label} Started\n\nTask: \`${taskKey}\``;
  return `${label} → ${taskKey}`;
}

// ─── Simulate Reaction ────────────────────────────────────────────────

export function formatSimulateReaction(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const rules = toArray(result.relevant_rules);
  const prediction = String(result.prediction || '');
  const lines: string[] = [];
  if (rules.length > 0) {
    lines.push(mode === 'verbose' ? '### Relevant Preferences' : 'Preferences:');
    rules.forEach((r: Record<string, unknown>) => {
      const stage = String(r.lifecycle_stage || 'draft');
      const text = String(r.rule_text || '');
      const tags = String(r.tags || '');
      lines.push(mode === 'verbose'
        ? `- ${stageStr(stage, 'verbose')} **${text}** [${tags}]`
        : `${stageStr(stage, 'default')} ${text} [${tags}]`);
    });
  }
  if (prediction) {
    lines.push(mode === 'verbose' ? `### Prediction\n${prediction}` : `Prediction: ${prediction}`);
  }
  if (lines.length === 0) lines.push('No relevant heuristics found.');
  if (mode === 'verbose') return `## Simulate Reaction\n\n${lines.join('\n')}`;
  return lines.join('\n');
}

// ─── Heuristics L0 ─────────────────────────────────────────────────────

export function formatHeuristicsL0(result: unknown, mode: OutputMode): string {
  const rules = toArray(result);
  if (mode === 'json') return JSON.stringify(result);
  if (rules.length === 0) return 'No confirmed rules found.';
  const lines = rules.map((r: Record<string, unknown>) => {
    const text = String(r.rule_text || '');
    const tags = String(r.tags || '');
    return mode === 'verbose'
      ? `- ${stageStr('confirmed', 'verbose')} **${text}** [${tags}]`
      : `[confirmed] ${text} [${tags}]`;
  });
  if (mode === 'verbose') return `## Confirmed Rules (${rules.length})\n\n${lines.join('\n')}`;
  return `Rules (${rules.length}):\n${lines.join('\n')}`;
}

// ─── Session Context ───────────────────────────────────────────────────

export function formatSessionContext(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const messages = toArray(result.messages);
  const overview = String(result.latest_archive_overview || '');
  const lines: string[] = [];
  if (overview) lines.push(mode === 'verbose' ? `### Archive Overview\n${overview}` : `Archive: ${overview}`);
  if (messages.length > 0) {
    lines.push(mode === 'verbose' ? '### Messages' : 'Messages:');
    messages.forEach((m: Record<string, unknown>) => {
      const role = String(m.role || '');
      const content = String(m.content || '');
      lines.push(mode === 'verbose'
        ? `- **[${role}]** ${content}`
        : `[${role}] ${content.substring(0, 200)}`);
    });
  }
  if (lines.length === 0) lines.push('No context available.');
  if (mode === 'verbose') return `## Session Context\n\n${lines.join('\n')}`;
  return lines.join('\n');
}

// ─── Session Delete ────────────────────────────────────────────────────

export function formatSessionDelete(sessionId: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  if (mode === 'verbose') return `## Session Deleted\n\n✓ Session \`${sessionId}\` deleted`;
  return `Deleted ${sessionId}`;
}

// ─── Memory Management ────────────────────────────────────────────────

export function formatCiteMemories(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const ep = Number(result.cited_episodes || 0);
  const fa = Number(result.cited_facts || 0);
  if (mode === 'verbose') {
    return `## Citation Recorded\n\n✓ ${ep} episode(s) cited\n✓ ${fa} fact(s) cited`;
  }
  return `Cited ${ep} ep + ${fa} facts`;
}

export function formatExportMemories(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const md = String(result.markdown || '');
  const factCount = Number(result.fact_count || 0);
  const ruleCount = Number(result.rule_count || 0);
  if (mode === 'verbose') {
    const lines = ['## Memory Export'];
    lines.push(`Facts: ${factCount}, Rules: ${ruleCount}`);
    return md || lines.join('\n');
  }
  // Default mode: output markdown with comment-line summary header
  // Comment lines (<!-- ... -->) survive markdown parsing but won't interfere
  // with pipeline round-trip: memfuse export-memories | memfuse import-memories
  if (md) return `<!-- Export: ${factCount} facts, ${ruleCount} rules -->\n${md}`;
  return `Export: ${factCount} facts, ${ruleCount} rules (empty content)`;
}

export function formatImportMemories(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const updated = Number(result.updated_facts || 0);
  const retracted = Number(result.retracted_facts || 0);
  const total = Number(result.total_imported || 0);
  if (mode === 'verbose') {
    return `## Import Complete\n\n✓ Total parsed: ${total}\n✓ Updated: ${updated}\n✓ Retracted: ${retracted}`;
  }
  return `Imported ${total} (upd:${updated} ret:${retracted})`;
}

// ─── Heuristic ────────────────────────────────────────────────────────

export function formatConfirmRule(result: Record<string, unknown>, ruleId: string, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const confirmed = Boolean(result.user_confirmed);
  if (mode === 'verbose') {
    return confirmed ? `## Rule Confirmed\n\n★ Rule \`${ruleId}\` confirmed` : `## Rule Not Confirmed\n\n○ Rule \`${ruleId}\` not confirmed`;
  }
  return `${ruleId} ${confirmed ? 'confirmed' : 'not confirmed'}`;
}

// ─── Skills ───────────────────────────────────────────────────────────

export function formatSkillsList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).skills || results
    : results;
  const skills = toArray(payload);
  if (skills.length === 0) return 'No skills found.';
  if (mode === 'verbose') {
    return `## Skills (${skills.length})\n\n${skills.map(s => `- \`${String(s.name || s.skill_name || '')}\` ${String(s.uri || '')}`).join('\n')}`;
  }
  return skills.map(s => `${String(s.name || s.skill_name || '')} ${String(s.uri || '')}`).join('\n');
}

export function formatSkillAdded(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  if (mode === 'verbose') return '## Skill Added\n\n✓ Skill registered successfully';
  return 'Skill added.';
}

// ─── System / Health ──────────────────────────────────────────────────

export function formatHealth(result: Record<string, unknown>, serverUrl: string, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const status = String(result.status || 'unknown');
  const version = String(result.version || '');
  if (mode === 'verbose') {
    const lines = ['## MemFuse Health'];
    lines.push(`Status: ${status}`);
    if (version) lines.push(`Version: ${version}`);
    lines.push(`Server: ${serverUrl}`);
    return lines.join('\n');
  }
  return `${status} v${version} @${serverUrl}`;
}

export function formatSystemStatus(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const lines: string[] = [];
  for (const [key, val] of Object.entries(result)) {
    if (typeof val === 'object' && val !== null) {
      lines.push(`${key}: ${JSON.stringify(val)}`);
    } else {
      lines.push(`${key}: ${val}`);
    }
  }
  if (mode === 'verbose') return `## System Status\n\n${lines.join('\n')}`;
  return lines.join('\n');
}

export function formatObserverStatus(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const lines: string[] = [];
  for (const [key, val] of Object.entries(result)) {
    if (typeof val === 'object' && val !== null) {
      lines.push(`${key}: ${JSON.stringify(val)}`);
    } else {
      lines.push(`${key}: ${val}`);
    }
  }
  if (mode === 'verbose') return `## Observer Status\n\n${lines.join('\n')}`;
  return lines.join('\n');
}

// ─── Session Archive ────────────────────────────────────────────────

export function formatSessionArchive(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const overview = String(result.overview || result.latest_archive_overview || '');
  const messages = toArray(result.messages);
  if (mode === 'verbose') {
    const lines = [`## Session Archive`];
    if (overview) lines.push(`Overview: ${overview}`);
    if (messages.length > 0) {
      lines.push('Messages:');
      messages.forEach((m: Record<string, unknown>) => {
        lines.push(`- **[${String(m.role || '')}]** ${String(m.content || '')}`);
      });
    }
    return lines.join('\n');
  }
  if (overview) return overview;
  if (messages.length > 0) return messages.map(m => `[${String(m.role || '')}] ${String(m.content || '').substring(0, 200)}`).join('\n');
  return 'No archive content.';
}

// ─── Resource Extended (Export/Import) ──────────────────────────────────

export function formatResourceExport(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const outputPath = String(result.output_path || result.path || '');
  if (mode === 'verbose') {
    const lines = ['## Resource Exported'];
    if (outputPath) lines.push(`✓ Exported to: \`${outputPath}\``);
    return lines.join('\n');
  }
  return `Exported → ${outputPath}`;
}

export function formatResourceImportResult(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const resourceId = String(result.resource_id || '');
  const taskKey = String(result.task_key || '');
  if (mode === 'verbose') {
    const lines = ['## Resource Imported'];
    if (resourceId) lines.push(`✓ Resource ID: \`${resourceId}\``);
    if (taskKey) lines.push(`Task: \`${taskKey}\``);
    return lines.join('\n');
  }
  return `Imported ${resourceId}${taskKey ? ` task:${taskKey}` : ''}`;
}

// ─── Code Symbols Formatting ────────────────────────────────────────────

export function formatCodeSymbolsList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).symbols || results
    : results;
  const symbols = toArray(payload);
  if (symbols.length === 0) return 'No code symbols found.';
  const lines = symbols.map((s: Record<string, unknown>) => {
    const id = String(s.view_id || s.id || '');
    const name = String(s.symbol_name || s.name || '');
    const uri = String(s.uri || s.canonical_uri || '');
    const count = Number(s.symbol_count || 0);
    return mode === 'verbose'
      ? `- \`${id}\` ${name} uri: \`${uri}\` symbols: ${count}`
      : `${id} ${name} ${uri} ${count}`;
  });
  if (mode === 'verbose') return `## Code Symbols (${symbols.length})\n\n${lines.join('\n')}`;
  return `Symbols (${symbols.length}):\n${lines.join('\n')}`;
}

export function formatCodeSymbolsSearch(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const results = toArray(result.items || result.results || result.symbols || result);
  if (results.length === 0) return 'No matching code symbols.';
  const lines = results.map((r: Record<string, unknown>) => {
    const name = String(r.name || r.symbol_name || '');
    const uri = String(r.uri || r.canonical_uri || '');
    const score = Number(r.score || 0).toFixed(2);
    return mode === 'verbose'
      ? `- **${name}** in \`${uri}\` (score: ${score})`
      : `${name} ${uri} (${score})`;
  });
  if (mode === 'verbose') return `## Code Symbol Search (${results.length})\n\n${lines.join('\n')}`;
  return `Search (${results.length}):\n${lines.join('\n')}`;
}

export function formatCodeSymbolsCreated(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.view_id || result.id || '');
  if (mode === 'verbose') return `## Code Symbols Created\n\n✓ View \`${id}\` created`;
  return `Created ${id}`;
}

export function formatCodeSymbolsDeleted(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const deleted = Boolean(result.deleted);
  if (mode === 'verbose') return deleted ? `## Code Symbols Deleted\n\n✓ View deleted` : `## Not Deleted`;
  return deleted ? 'Deleted' : 'Not deleted';
}

// ─── Workspace Write Ops ───────────────────────────────────────────────

export function formatWorkspaceWrite(op: string, uri: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const ok = Boolean(result.ok || result.success || result.created);
  if (mode === 'verbose') {
    return `## ${op} ${ok ? '✓' : '○'}\n\n${op} on \`${uri}\` ${ok ? 'succeeded' : 'failed'}`;
  }
  return `${op} ${ok ? 'ok' : 'failed'} ${uri}`;
}

// ─── Relations ─────────────────────────────────────────────────────────

export function formatRelationLink(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const from = String(result.from_uri || '');
  const to = String(result.to_uri || '');
  const type = String(result.relation_type || 'related');
  if (mode === 'verbose') return `## Link Created\n\n✓ \`${from}\` → \`${to}\` (${type})`;
  return `Linked ${from} → ${to} (${type})`;
}

export function formatRelationUnlink(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const from = String(result.from_uri || '');
  const to = String(result.to_uri || '');
  const type = String(result.relation_type || 'related');
  if (mode === 'verbose') return `## Link Removed\n\n✓ \`${from}\` − \`${to}\` (${type})`;
  return `Unlinked ${from} − ${to} (${type})`;
}

export function formatRelationList(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const relations = toArray(result.items || result.relations || result);
  if (relations.length === 0) return 'No relations found.';
  const lines = relations.map((r: Record<string, unknown>) => {
    const from = String(r.from_uri || '');
    const to = String(r.to_uri || '');
    const direction = String(r.direction || '');
    const peer = String(r.peer_uri || '');
    const type = String(r.relation_type || 'related');
    if (peer) {
      return mode === 'verbose'
        ? `- ${direction ? `${direction}: ` : ''}\`${peer}\` (${type})`
        : `${direction ? `${direction} ` : ''}${peer} (${type})`;
    }
    return mode === 'verbose'
      ? `- \`${from}\` → \`${to}\` (${type})`
      : `${from} → ${to} (${type})`;
  });
  if (mode === 'verbose') return `## Relations (${relations.length})\n\n${lines.join('\n')}`;
  return `Relations (${relations.length}):\n${lines.join('\n')}`;
}

// ─── Watch Operations ──────────────────────────────────────────────────

export function formatWatchList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).watches || results
    : results;
  const watches = toArray(payload);
  if (watches.length === 0) return 'No watches registered.';
  const lines = watches.map((w: Record<string, unknown>) => {
    const id = String(w.watch_id || w.id || '');
    const resourceId = String(w.resource_id || '');
    const status = String(w.status || 'unknown');
    return mode === 'verbose'
      ? `- \`${id}\` resource: \`${resourceId}\` status: ${status}`
      : `${id} ${resourceId} ${status}`;
  });
  if (mode === 'verbose') return `## Watches (${watches.length})\n\n${lines.join('\n')}`;
  return `Watches (${watches.length}):\n${lines.join('\n')}`;
}

export function formatWatchOp(label: string, resourceId: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const status = String(result.status || 'triggered');
  if (mode === 'verbose') {
    const lines = [`## ${label}`];
    if (resourceId) lines.push(`Resource: \`${resourceId}\``);
    lines.push(`Status: ${status}`);
    return lines.join('\n');
  }
  return `${label} ${resourceId || status} → ${status}`;
}

export function formatWatchDaemonStatus(action: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const running = Boolean(result.running || result.is_running);
  const lines: string[] = [];
  for (const [key, val] of Object.entries(result)) {
    lines.push(`${key}: ${typeof val === 'object' ? JSON.stringify(val) : val}`);
  }
  if (mode === 'verbose') return `## Watch Daemon (${action})\n\n${lines.join('\n')}`;
  return lines.join('\n');
}

// ─── Facts Write Operations ───────────────────────────────────────────

export function formatFactCreated(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.fact_id || result.id || '');
  if (mode === 'verbose') return `## Fact Created\n\n✓ Fact \`${id}\` created`;
  return `Created ${id}`;
}

export function formatFactSuperseded(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const oldId = String(result.old_fact_id || '');
  const newId = String(result.new_fact_id || result.fact_id || result.id || '');
  if (mode === 'verbose') return `## Fact Superseded\n\n✓ Old: \`${oldId}\` → New: \`${newId}\``;
  return `Superseded ${oldId} → ${newId}`;
}

export function formatFactRetracted(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.fact_id || result.id || '');
  const retracted = Boolean(result.retracted);
  if (mode === 'verbose') return retracted ? `## Fact Retracted\n\n✓ Fact \`${id}\` retracted` : `## Fact Not Retracted\n\n○ Fact \`${id}\` could not be retracted`;
  return `${id} ${retracted ? 'retracted' : 'retract failed'}`;
}

// ─── Fact Trace (§2.4 provenance) ────────────────────────────────────

export function formatFactTrace(factId: string, result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const fact = result.fact as Record<string, unknown> | undefined;
  const sourceEpisodes = toArray(result.source_episodes);
  const sourceAssertions = toArray(result.source_assertions);
  const lines: string[] = [];

  if (fact) {
    const subj = String(fact.subject || '');
    const pred = String(fact.predicate || '');
    const val = String(fact.display_value || '');
    const conf = Number(fact.confidence || 0).toFixed(2);
    const validFrom = String(fact.valid_from || '');
    const validTo = String(fact.valid_to || '');
    if (mode === 'verbose') {
      lines.push(`## Fact Provenance: \`${factId}\``);
      lines.push(`- **${subj}** → ${pred}: ${val} (confidence: ${conf})`);
      if (validFrom) lines.push(`  Valid from: ${validFrom}`);
      if (validTo) lines.push(`  Valid to: ${validTo}`);
    } else {
      lines.push(`Fact: ${subj} → ${pred}: ${val} (${conf})`);
      if (validFrom) lines.push(`  from: ${validFrom}`);
      if (validTo) lines.push(`  to: ${validTo}`);
    }
  } else {
    lines.push(mode === 'verbose' ? `## Fact Not Found: \`${factId}\`` : `Fact ${factId} not found`);
  }

  if (sourceEpisodes.length > 0) {
    lines.push(mode === 'verbose' ? '### Source Episodes' : 'Sources:');
    sourceEpisodes.forEach((ep: Record<string, unknown>) => {
      const id = String(ep.episode_id || ep.id || '');
      const summary = String(ep.summary || '');
      const created = String(ep.created_at || '');
      const dateStr = created ? new Date(created).toLocaleDateString() : '';
      if (mode === 'verbose') lines.push(`- **[${id}]** ${summary} (created: ${dateStr})`);
      else lines.push(`${id} ${summary} ${dateStr}`);
    });
  } else {
    lines.push(mode === 'verbose' ? '### Source Episodes\nNo source episodes found.' : 'Sources: none');
  }

  if (sourceAssertions.length > 0) {
    lines.push(mode === 'verbose' ? '### Source Assertions' : 'Assertions:');
    sourceAssertions.forEach((a: Record<string, unknown>) => {
      const assertionId = String(a.assertion_id || a.id || '');
      const subj = String(a.subject || '');
      const pred = String(a.predicate || '');
      const op = String(a.operation || '');
      const conf = Number(a.confidence || 0).toFixed(2);
      if (mode === 'verbose') lines.push(`- **[${assertionId}]** ${subj} ${pred} (${op}, confidence: ${conf})`);
      else lines.push(`${assertionId} ${subj} ${pred} ${op} (${conf})`);
    });
  }

  return lines.join('\n');
}

// ─── Heuristic Rule CRUD Formatting ───────────────────────────────────

export function formatHeuristicRuleCreated(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.rule_id || result.id || '');
  const stage = String(result.lifecycle_stage || 'draft');
  if (mode === 'verbose') return `## Rule Created\n\n${stageStr(stage, 'verbose')} Rule \`${id}\` created (${stage})`;
  return `Created ${id} [${stage}]`;
}

export function formatHeuristicRuleList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).rules || results
    : results;
  const rules = toArray(payload);
  if (rules.length === 0) return 'No rules found.';
  const lines = rules.map((r: Record<string, unknown>) => {
    const id = String(r.rule_id || r.id || '');
    const text = String(r.rule_text || '');
    const stage = String(r.lifecycle_stage || 'draft');
    const tags = String(r.tags || '');
    return mode === 'verbose'
      ? `- ${stageStr(stage, 'verbose')} **${text}** [${tags}] (ID: \`${id}\`)`
      : `${id} ${stageStr(stage, 'default')} ${text} [${tags}]`;
  });
  if (mode === 'verbose') return `## Heuristic Rules (${rules.length})\n\n${lines.join('\n')}`;
  return `Rules (${rules.length}):\n${lines.join('\n')}`;
}

export function formatHeuristicRuleDetail(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.rule_id || result.id || '');
  const text = String(result.rule_text || '');
  const stage = String(result.lifecycle_stage || 'draft');
  const tags = String(result.tags || '');
  const evidence = Number(result.evidence_count || 0);
  if (mode === 'verbose') {
    const lines = [`## Rule Detail: \`${id}\``];
    lines.push(`Text: ${text}`);
    lines.push(`Stage: ${stageStr(stage, 'verbose')} (${stage})`);
    lines.push(`Tags: ${tags}`);
    lines.push(`Evidence: ${evidence}`);
    return lines.join('\n');
  }
  return `${id} ${stageStr(stage, 'default')} ${text} [${tags}] evidence:${evidence}`;
}

export function formatHeuristicRulePromoted(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.rule_id || result.id || '');
  const stage = String(result.lifecycle_stage || result.new_stage || '');
  if (mode === 'verbose') return `## Rule Promoted\n\n${stageStr(stage, 'verbose')} Rule \`${id}\` promoted to ${stage}`;
  return `${id} promoted → ${stage}`;
}

export function formatHeuristicInstanceCreated(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.instance_id || result.id || '');
  if (mode === 'verbose') return `## Instance Created\n\n✓ Instance \`${id}\` created`;
  return `Created ${id}`;
}

export function formatHeuristicInstanceList(results: unknown, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(results);
  const payload = results && typeof results === 'object' && !Array.isArray(results)
    ? (results as Record<string, unknown>).items || (results as Record<string, unknown>).instances || results
    : results;
  const instances = toArray(payload);
  if (instances.length === 0) return 'No instances found.';
  const lines = instances.map((i: Record<string, unknown>) => {
    const id = String(i.instance_id || i.id || '');
    const ruleId = String(i.rule_id || '');
    const ctx = String(i.context || i.context_summary || '').substring(0, 50);
    return mode === 'verbose'
      ? `- \`${id}\` rule: \`${ruleId}\` context: ${ctx}`
      : `${id} rule:${ruleId} ${ctx}`;
  });
  if (mode === 'verbose') return `## Heuristic Instances (${instances.length})\n\n${lines.join('\n')}`;
  return `Instances (${instances.length}):\n${lines.join('\n')}`;
}

export function formatHeuristicInstanceDetail(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const id = String(result.instance_id || result.id || '');
  const ruleId = String(result.rule_id || '');
  const context = String(result.context || '');
  const evidence = String(result.evidence || '');
  if (mode === 'verbose') {
    const lines = [`## Instance Detail: \`${id}\``];
    lines.push(`Rule: ${ruleId}`);
    if (context) lines.push(`Context: ${context}`);
    if (evidence) lines.push(`Evidence: ${evidence}`);
    return lines.join('\n');
  }
  return `${id} rule:${ruleId}`;
}

export function formatHeuristicRetrieve(result: Record<string, unknown>, mode: OutputMode): string {
  if (mode === 'json') return JSON.stringify(result);
  const rules = toArray(result.rules || result.heuristics);
  if (rules.length === 0) return 'No matching heuristics.';
  const lines = rules.map((r: Record<string, unknown>) => {
    const id = String(r.rule_id || r.id || '');
    const text = String(r.rule_text || '');
    const stage = String(r.lifecycle_stage || 'draft');
    const tags = String(r.tags || '');
    const weight = Number(r.aggregate_weight || r.weight || 0).toFixed(2);
    return mode === 'verbose'
      ? `- ${stageStr(stage, 'verbose')} **${text}** [${tags}] (weight: ${weight})`
      : `${id} ${stageStr(stage, 'default')} ${text} [${tags}] w:${weight}`;
  });
  if (mode === 'verbose') return `## Retrieved Heuristics (${rules.length})\n\n${lines.join('\n')}`;
  return `Retrieve (${rules.length}):\n${lines.join('\n')}`;
}

// ─── Internal Helpers ─────────────────────────────────────────────────

function confStr(conf: number, mode: OutputMode): string {
  if (mode === 'verbose') {
    const marker = conf >= 0.9 ? '✓' : conf >= 0.7 ? '~' : '?';
    return `${marker}`;
  }
  return conf.toFixed(2);
}

export function stageStr(stage: string, mode: OutputMode): string {
  if (mode === 'verbose') {
    return stage === 'confirmed' ? '★' : stage === 'candidate' ? '◆' : '○';
  }
  return stage === 'confirmed' ? '[confirmed]' : stage === 'candidate' ? '[candidate]' : '[draft]';
}

export { toArray };

function capitalize(s: string): string {
  return s.charAt(0).toUpperCase() + s.slice(1);
}
