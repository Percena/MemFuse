#!/usr/bin/env node
/**
 * MemFuse MCP Server — Universal agent memory via MCP protocol
 *
 * Works with Claude Code, Codex, Cursor, and any MCP-compatible agent.
 * Architecture: MCP server → MemFuse Server (single-backend, Phase 0 simplification).
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js';
import { StdioServerTransport } from '@modelcontextprotocol/sdk/server/stdio.js';
import { z } from 'zod';
import { callBackend } from '../shared/http.js';
import { CanvasRouter } from '../shared/router.js';
import { loadConfig } from '../shared/config.js';
import { sanitizeMemoryText } from '../shared/privacy.js';
import { extractErrorMessage, toArray } from '../shared/utils.js';

const config = loadConfig();
const router = new CanvasRouter(config);

const PATHS = {
  OBSERVE: '/sessions',              // base — append `/{sessionId}/observations` at call site
  SEARCH: '/v1/memory:search',
  FACTS: '/facts',
  CONTEXT_RESOLVE: '/context/resolve',
  OBSERVATIONS_GET: '/sessions',     // base — append `/{sessionId}/context` at call site
  CONSOLIDATE: '/sessions',          // base — append `/{sessionId}/commit` at call site
  EPISODES: '/episodes',
  RESOURCES: '/resources',           // POST to create, GET to list
  TASKS: '/tasks',                   // GET task status
  LS: '/ls',
  READ: '/read',
  ABSTRACT: '/abstract',
  OVERVIEW: '/overview',
  GLOB: '/glob',
  HEURISTICS_SIMULATE: '/heuristics/simulate-reaction',
  HEURISTICS_L0: '/heuristics/l0-confirmed',
  GREP: '/grep',
  CITE: '/memories/cite',
  RELATIONS: '/relations',
} as const;

// ─── Orphan process protection ─────────────────────────────────────────

function startParentHeartbeat(): void {
  const parentPid = process.ppid;
  if (!parentPid || parentPid <= 1) return;
  const interval = setInterval(() => {
    try { process.kill(parentPid, 0); }
    catch (_) {
      clearInterval(interval);
      console.error('MemFuse MCP: Parent process exited, shutting down.');
      process.exit(0);
    }
  }, 30000);
}

// ─── MCP Server ────────────────────────────────────────────────────────

const server = new McpServer({ name: 'memfuse', version: '0.1.0' });

function unwrapData(result: Record<string, unknown>): Record<string, unknown> {
  const data = result.data;
  if (data && typeof data === 'object' && !Array.isArray(data)) {
    return data as Record<string, unknown>;
  }
  return result;
}

function record(val: unknown): Record<string, unknown> {
  return val && typeof val === 'object' && !Array.isArray(val)
    ? val as Record<string, unknown>
    : {};
}

// ─── Tool 0: memfuse_guide ────────────────────────────────────────────

server.registerTool('memfuse_guide', {
  description: 'IMPORTANT: Read this first. Returns a concise guide on how to use all MemFuse MCP tools together for token-efficient memory retrieval.',
  inputSchema: z.object({ topic: z.string().optional().describe('Optional: a specific topic you want guidance on') }),
}, async () => {
  return {
    content: [{
      type: 'text',
      text: [
        '# MemFuse MCP Tools — Quick Guide',
        '',
        '## 3-Layer Progressive Workflow (saves ~10x tokens)',
        '',
        '1. **search** → `search_memories(query="...")` → compact index (~50-100 tokens/result)',
        '2. **timeline** → `timeline(episode_id="...")` → chronological context (~200-400 tokens)',
        '3. **detail** → `get_observations(episode_ids=["..."])` → full details only for selected IDs (~500-1000 tokens/result)',
        '',
        '## All Available Tools',
        '',
        '| Tool | Purpose | Typical tokens |',
        '|------|---------|----------------|',
        '| `search_memories` | Search memory by keyword/semantic query | 50-100/result |',
        '| `timeline` | Get chronological context around an episode | 200-400 |',
        '| `get_observations` | Fetch full episode details by IDs | 500-1000/result |',
        '| `resolve_context` | One-shot context injection (facts + episodes) | 500-1500 |',
        '| `ls` | List a resource directory pointed to by memory | ~50-200 |',
        '| `read` | Read a specific resource/file | variable |',
        '| `abstract` | Read compact L0 summary text | ~50-150 |',
        '| `overview` | Read L1 overview text | ~150-400 |',
        '| `glob` | Narrow within a resource tree by pattern | ~50-200 |',
        '| `grep` | Literal verification after memory narrows the scope | ~50-300 |',
        '| `store_observation` | Store a new observation | ~200 |',
        '| `commit_session` | Commit session to trigger memory consolidation | ~100 |',
        '| `add_resource` | Add a resource (localfs/git/git_url/inline) | ~100 |',
        '| `add_repo` | Add a git repo (local path or remote URL) | ~100 |',
        '| `add_resource_inline` | Add inline content directly | ~100 |',
        '| `task_status` | Check async task progress | ~50 |',
        '| `list_facts` | List all active facts for current user | ~100-200 |',
        '| `inject_context` | Alias for resolve_context with behavioral heuristics | 500-1500 |',
        '| `simulate_reaction` | Simulate user reaction to proposed action | ~200-400 |',
        '| `heuristics_l0_confirmed` | Get top confirmed rules for session-start | ~100-300 |',
        '| `heuristics_confirm_rule` | Mark a heuristic rule as user-confirmed | ~50 |',
        '| `cite_memories` | Record useful episodes/facts (improves ranking) | ~50 |',
        '| `export_memories` | Export facts and rules as editable Markdown | variable |',
        '| `import_memories` | Import memories from edited Markdown | ~100 |',
        '',
        '## When to Use Each Tool',
        '',
        '- **Session start** → `resolve_context` with a query for auto-injection of relevant context',
        '- **Need specific past info** → start with `search_memories`, then `timeline` or `get_observations`',
        '- **Memory hit points to a resource** → use `ls`, `abstract`, `overview`, or `read` to dig into that URI',
        '- **Need to narrow inside a resource tree** → use `glob` first, then `grep` or `read`',
        '- **Learned something important** → `store_observation` to persist it',
        '- **Check known facts** → `list_facts`',
        '- **Want full context quickly** → `resolve_context` (combines facts + episodes)',
        '- **Used memory in your answer** → `cite_memories` to boost those memories for future recall',
        '- **Review/edit stored memories** → `export_memories`, edit, then `import_memories`',
        '',
        '## Tips',
        '',
        '- Always start broad (`search_memories`), then narrow down with `get_observations`',
        '- `resolve_context` is cheapest for a quick overview but less targeted',
        '- Only call `get_observations` for IDs that look relevant from search/timeline',
        '- When a memory hit returns a `mfs://...` pointer, treat it as a dig handle rather than a final answer',
        '- Store observations with specific file paths and reasoning for best future recall',
        '- Private content wrapped in `<private>` tags is automatically stripped before storage',
      ].join('\n'),
    }],
  };
});

// ─── Tool 1: search_memories ──────────────────────────────────────────

server.registerTool('search_memories', {
  description: 'Search persistent memory across sessions. Returns compact index with episode IDs, summaries, and scores (~50-100 tokens/result). Use this first, then call timeline or get_observations for IDs that look relevant.',
  inputSchema: z.object({
    query: z.string().describe('Search query (keywords or natural language)'),
    limit: z.number().default(10).describe('Maximum number of results (default: 10)'),
    strategy: z.enum(['precision', 'diverse', 'recent', 'comprehensive']).default('precision').describe('Search strategy preset (default: precision)'),
  }),
}, async ({ query, limit, strategy }) => {
  try {
    const result = await callBackend('POST', PATHS.SEARCH, {
      user_id: config.userId, query, limit: limit ?? 10, strategy,
    }, router) as Record<string, unknown>;

    return { content: [{ type: 'text', text: formatSearchResults(result) }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error searching memories: ${msg}` }], isError: true };
  }
});

// ─── Tool 2: timeline ──────────────────────────────────────────────────

server.registerTool('timeline', {
  description: 'Get chronological context around a specific episode. Shows what happened before and after. Cheaper than get_observations (~200-400 tokens). Use after search_memories to narrow down.',
  inputSchema: z.object({
    episode_id: z.string().describe('The episode ID to anchor the timeline around'),
    direction: z.enum(['before', 'after', 'both']).default('both').describe('Direction: "before", "after", or "both" (default: "both")'),
    radius: z.number().default(3).describe('Number of episodes on each side (default: 3)'),
  }),
}, async ({ episode_id, direction, radius }) => {
  try {
    const params = new URLSearchParams({
      direction: direction ?? 'both',
      radius: String(radius ?? 3),
    });
    const result = await callBackend(
      'GET',
      `${PATHS.EPISODES}/${encodeURIComponent(episode_id)}/timeline?${params.toString()}`,
      null,
      router,
    ) as Record<string, unknown>;

    const episodes = toArray(result.episodes);
    const facts = toArray(result.current_facts);

    return { content: [{ type: 'text', text: formatTimeline(episode_id, episodes, facts) }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error getting timeline: ${msg}` }], isError: true };
  }
});

// ─── Tool 3: get_observations ──────────────────────────────────────────

server.registerTool('get_observations', {
  description: 'Fetch full episode details by IDs. Use only for episodes that look relevant from search_memories or timeline results (~500-1000 tokens/result). This is the most expensive retrieval — always filter first.',
  inputSchema: z.object({
    episode_ids: z.array(z.string()).describe('List of episode IDs to fetch full details for'),
  }),
}, async ({ episode_ids }) => {
  try {
    if (episode_ids.length === 0) {
      return { content: [{ type: 'text', text: 'No episode IDs provided. Pass IDs from search_memories or timeline results.' }] };
    }

    const details = await Promise.all(episode_ids.map(async (id) => {
      try {
        return await callBackend('GET', `${PATHS.EPISODES}/${encodeURIComponent(id)}`, null, router) as Record<string, unknown>;
      } catch (_) { return { id, error: 'unavailable' }; }
    }));
    return { content: [{ type: 'text', text: formatObservations(details) }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error fetching observations: ${msg}` }], isError: true };
  }
});

// ─── Tool 4: resolve_context ───────────────────────────────────────────

server.registerTool('resolve_context', {
  description: 'Resolve memory context for a query. Returns relevant facts, recent updates, episodic memories, and behavioral heuristics in one call (~500-1500 tokens). Use at session start or when you need a quick overview. IMPORTANT for Codex users: call this tool at the start of every new task to get task-relevant context, since Codex lacks UserPromptSubmit hooks.',
  inputSchema: z.object({
    query: z.string().describe('Query to resolve context for'),
    budget: z.number().default(1500).describe('Token budget (default: 1500). Higher budgets return more detail.'),
    strategy: z.enum(['precision', 'diverse', 'recent', 'comprehensive']).default('precision').describe('Search strategy: precision (default, relevance-only), diverse (relevance+MMR diversity reranking), recent (enhanced recency boost), comprehensive (budget×2 for maximum recall)'),
    at_time: z.string().optional().describe('Point-in-time query (ISO 8601): return facts that were effective at this timestamp. Example: "2026-04-01T00:00:00Z" returns facts valid on April 1.'),
    session_id: z.string().optional().describe('Session ID to scope the overlay (recent unconsolidated turns). Pass the host session ID so MCP calls and lifecycle hooks share the same session.'),
  }),
}, async ({ query, budget, strategy, at_time, session_id }) => {
  try {
    const sessionId = session_id || config.sessionId || 'default';
    const result = await callBackend('POST', PATHS.CONTEXT_RESOLVE, {
      user_id: config.userId, session_id: sessionId, query, token_budget: budget ?? 1500, strategy, at_time,
    }, router) as Record<string, unknown>;

    const sections = result.sections as Record<string, unknown> ?? { current_facts: [], recent_updates: [], relevant_history: [] };

    const facts = toArray(sections.current_facts);
    const history = toArray(sections.relevant_history);
    const recent = toArray(sections.recent_updates);
    const heuristics = toArray(sections.behavioral_heuristics as unknown);
    const formatted = result.rendered_markdown || '';

    if (formatted && typeof formatted === 'string') {
      return { content: [{ type: 'text', text: formatted }] };
    }

    const parts: string[] = [];

    if (facts.length > 0) {
      parts.push('## Active Facts');
      parts.push(facts.map((f: Record<string, unknown>) => {
        const conf = Number(f.confidence || 0);
        const marker = conf >= 0.9 ? '✓' : conf >= 0.7 ? '~' : '?';
        const subj = String(f.subject || '');
        const pred = String(f.predicate || '');
        const val = String(f.display_value || '');
        return `- ${marker} **${subj}** → ${pred}: ${val}`;
      }).join('\n'));
    }

    if (recent.length > 0) {
      parts.push('## Recent Updates');
      parts.push(recent.map((r: unknown) => `- ${r}`).join('\n'));
    }

    if (history.length > 0) {
      parts.push('## Relevant Episodes');
      parts.push(history.map((ep: Record<string, unknown>) => {
        const id = String(ep.episode_id || '');
        const summary = String(ep.summary || '');
        const score = Number(ep.score || 0);
        return `- **[${id}]** ${summary} (score: ${score.toFixed(2)})`;
      }).join('\n'));
    }

    if (heuristics.length > 0) {
      parts.push('## Behavioral Heuristics');
      parts.push(heuristics.map((h: Record<string, unknown>) => {
        const stage = String(h.lifecycle_stage || 'draft');
        const marker = stage === 'confirmed' ? '★' : stage === 'candidate' ? '◆' : '○';
        const text = String(h.rule_text || '');
        const tags = String(h.tags || '');
        return `- ${marker} **${text}** [${tags}]`;
      }).join('\n'));
    }

    const contextText = parts.length > 0 ? parts.join('\n\n') : 'No relevant context found.';
    return { content: [{ type: 'text', text: contextText }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error resolving context: ${msg}` }], isError: true };
  }
});

// ─── Tool: inject_context (alias for resolve_context, roadmap §2.3) ────

server.registerTool('inject_context', {
  description: 'Alias for resolve_context. Inject memory context including behavioral heuristics. Use when you need task-relevant preferences and rules before taking action.',
  inputSchema: z.object({
    query: z.string().describe('Query or scenario to resolve context for'),
    budget: z.number().default(1500).describe('Token budget (default: 1500)'),
    strategy: z.enum(['precision', 'diverse', 'recent', 'comprehensive']).default('precision').describe('Search strategy preset'),
    session_id: z.string().optional().describe('Session ID to scope the overlay (defaults to MEMFUSE_SESSION_ID or "default")'),
  }),
}, async ({ query, budget, strategy, session_id }) => {
  try {
    // Delegate to resolve_context handler
    const sessionId = session_id || config.sessionId || 'default';
    const result = await callBackend('POST', PATHS.CONTEXT_RESOLVE, {
      user_id: config.userId, session_id: sessionId, query, token_budget: budget ?? 1500, strategy,
    }, router) as Record<string, unknown>;
    const formatted = result.rendered_markdown || '';
    if (formatted && typeof formatted === 'string') {
      return { content: [{ type: 'text', text: formatted }] };
    }
    return { content: [{ type: 'text', text: 'No relevant context found.' }] };
  } catch (err: unknown) {
    const msg = err instanceof Error ? err.message : String(err);
    return { content: [{ type: 'text', text: `inject_context error: ${msg}` }], isError: true };
  }
});

// ─── Tool: simulate_reaction (L2 heuristic prediction, roadmap §2.3) ──

server.registerTool('simulate_reaction', {
  description: 'Simulate the user\'s likely reaction to a proposed action based on learned heuristic rules (L2 injection). Returns relevant rules and a prediction summary. Use before risky or controversial actions.',
  inputSchema: z.object({
    scenario: z.string().describe('Description of the proposed action or scenario to evaluate'),
    tags: z.array(z.string()).optional().describe('Optional tags to narrow rule search (e.g., domain:backend, phase:production)'),
  }),
}, async ({ scenario, tags }) => {
  try {
    const result = await callBackend('POST', PATHS.HEURISTICS_SIMULATE, {
      scenario, tags: tags ?? [], user_id: config.userId,
    }, router) as Record<string, unknown>;
    const rules = toArray(result.relevant_rules as unknown);
    const prediction = String(result.prediction || '');
    const summary = String(result.rules_summary || '');

    const parts: string[] = [];
    if (rules.length > 0) {
      parts.push('## Relevant Learned Preferences');
      parts.push(rules.map((r: Record<string, unknown>) => {
        const stage = String(r.lifecycle_stage || 'draft');
        const marker = stage === 'confirmed' ? '★' : stage === 'candidate' ? '◆' : '○';
        const text = String(r.rule_text || '');
        const counterExamples = toArray(r.counter_examples as unknown);
        const ceStr = counterExamples.length > 0 ? ` (except: ${counterExamples.map((c: unknown) => String(c)).join('; ')})` : '';
        return `- ${marker} **${text}**${ceStr}`;
      }).join('\n'));
    }
    if (prediction) {
      parts.push(`**Prediction**: ${prediction}`);
    }
    if (summary) {
      parts.push(`**Rule summary**: ${summary}`);
    }

    const text = parts.length > 0 ? parts.join('\n\n') : 'No relevant heuristic rules found for this scenario.';
    return { content: [{ type: 'text', text }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error simulating reaction: ${msg}` }], isError: true };
  }
});

// ─── Tool: heuristics_l0_confirmed (L0 session-start injection, roadmap §2.3) ──

server.registerTool('heuristics_l0_confirmed', {
  description: 'Get top confirmed heuristic rules for L0 session-start injection. Returns the highest-priority rules that should always be visible at the start of a session.',
  inputSchema: z.object({
    max_rules: z.number().default(5).describe('Maximum number of rules to return (default: 5)'),
  }),
}, async ({ max_rules }) => {
  try {
    const result = await callBackend('POST', PATHS.HEURISTICS_L0, {
      user_id: config.userId, max_rules: max_rules ?? 5,
    }, router) as unknown;
    const rules = toArray(result as unknown);
    if (rules.length === 0) {
      return { content: [{ type: 'text', text: 'No confirmed heuristic rules found.' }] };
    }
    const text = rules.map((h: Record<string, unknown>) => {
      const marker = '★';
      const ruleText = String(h.rule_text || '');
      const tags = String(h.tags || '');
      const counterExamples = toArray(h.counter_examples as unknown);
      const ceStr = counterExamples.length > 0 ? ` ⚠️ except: ${counterExamples.map(String).join('; ')}` : '';
      return `- ${marker} **${ruleText}** [${tags}]${ceStr}`;
    }).join('\n');
    return { content: [{ type: 'text', text: `### Top Confirmed Rules\n${text}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error: ${msg}` }], isError: true };
  }
});

// ─── Tool: heuristics_confirm_rule (roadmap §5.4 user-confirmed exemption) ──

server.registerTool('heuristics_confirm_rule', {
  description: 'Mark a heuristic rule as user-confirmed (roadmap §5.4). User-confirmed rules are exempt from automatic decay, distinct from lifecycle_stage "confirmed" which is reached via auto-promotion. Use when a user explicitly validates a rule.',
  inputSchema: z.object({
    rule_id: z.string().describe('The rule_id to mark as user-confirmed'),
  }),
}, async ({ rule_id }) => {
  try {
    const result = await callBackend('POST', `/heuristics/rules/${rule_id}/confirm`, {
      user_id: config.userId,
    }, router) as Record<string, unknown>;
    const confirmed = Boolean(result.user_confirmed);
    return { content: [{ type: 'text', text: `Rule ${rule_id} ${confirmed ? 'confirmed' : 'not confirmed'}. User-confirmed rules are exempt from automatic decay.` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error: ${msg}` }], isError: true };
  }
});

server.registerTool('store_observation', {
  description: 'Store an observation in persistent memory. Use when you discover something important that should be remembered across sessions. Private content in <private> tags is automatically stripped.',
  inputSchema: z.object({
    tool_name: z.string().describe('Name of the tool or observation type (e.g., "Decision", "BugFix", "Discovery")'),
    tool_input: z.string().optional().describe('Input or context of the observation'),
    tool_output: z.string().optional().describe('Output or result of the observation'),
    content: z.string().optional().describe('Full observation content (optional, auto-generated if not provided)'),
    metadata: z.record(z.string(), z.unknown()).optional().describe('Structured metadata (tool_type, summary, outcome, etc.)'),
    session_id: z.string().optional().describe('Session ID to store the observation in. Pass the host session ID so MCP writes and hook-captured observations land in the same session.'),
  }),
}, async ({ tool_name, tool_input, tool_output, content, metadata, session_id }) => {
  try {
    const safeInput = sanitizeMemoryText(tool_input || '');
    const safeOutput = sanitizeMemoryText(tool_output || '');
    const obsContent = sanitizeMemoryText(content || buildContent(tool_name, safeInput, safeOutput));
    const sessionId = session_id || config.sessionId || 'default';

    // POST /sessions/{sessionId}/observations
    const result = await callBackend('POST', `${PATHS.OBSERVE}/${sessionId}/observations`, {
      tool_name, tool_input: safeInput, tool_output: safeOutput,
      content: obsContent, platform: 'mcp', metadata: metadata || undefined,
    }, router) as Record<string, unknown>;

    const turnId = String(result.turn_id || 'stored');
    const jobId = result.job_id || '';
    const synced = result.synced !== false;

    return {
      content: [{
        type: 'text',
        text: `Observation stored.\nID: ${turnId}${jobId ? `\nConsolidation job: ${jobId}` : ''}\nSynced: ${synced ? 'Yes' : 'Pending (will sync when server available)'}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error storing observation: ${msg}` }], isError: true };
  }
});

// ─── Tool 5a: relation helpers ──────────────────────────────────────────

server.registerTool('link_relations', {
  description: 'Create a relation between two MemFuse URIs. Use this when a memory, resource, skill, or session should point to another URI.',
  inputSchema: z.object({
    from_uri: z.string().describe('Source MemFuse URI'),
    to_uri: z.string().describe('Target MemFuse URI'),
    relation_type: z.string().default('references').describe('Relation type (default: references)'),
  }),
}, async ({ from_uri, to_uri, relation_type }) => {
  try {
    await callBackend('POST', PATHS.RELATIONS, {
      from_uri,
      to_uri,
      relation_type: relation_type || 'references',
    }, router);
    return { content: [{ type: 'text', text: `Relation linked: ${from_uri} --${relation_type || 'references'}--> ${to_uri}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error linking relation: ${msg}` }], isError: true };
  }
});

server.registerTool('list_relations', {
  description: 'List relations for a MemFuse URI.',
  inputSchema: z.object({
    uri: z.string().describe('MemFuse URI whose inbound/outbound relations should be listed'),
    limit: z.number().default(20).describe('Maximum number of relations to return'),
  }),
}, async ({ uri, limit }) => {
  try {
    const params = new URLSearchParams({ uri, limit: String(limit ?? 20) });
    const result = await callBackend('GET', `${PATHS.RELATIONS}?${params.toString()}`, null, router) as Record<string, unknown>;
    const relations = toArray(result.relations || result);
    if (relations.length === 0) {
      return { content: [{ type: 'text', text: 'No relations found.' }] };
    }
    const lines = relations.map((relation) => {
      const direction = String(relation.direction || '');
      const peer = String(relation.peer_uri || relation.to_uri || relation.from_uri || '');
      const type = String(relation.relation_type || 'references');
      return `- ${direction ? `${direction} ` : ''}${type}: ${peer}`;
    });
    return { content: [{ type: 'text', text: `Relations for ${uri}:\n${lines.join('\n')}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error listing relations: ${msg}` }], isError: true };
  }
});

// ─── Tool 5b: commit_session ────────────────────────────────────────────

server.registerTool('commit_session', {
  description: 'Commit the current session to trigger memory consolidation. This archives session messages and runs the background memory pipeline to extract episodes and facts. Use this when you want to ensure your session observations are consolidated into persistent memory, especially on Codex which lacks a SessionEnd event.',
  inputSchema: z.object({
    reason: z.string().optional().describe('Optional reason for the commit (e.g., "end of task", "manual checkpoint")'),
    session_id: z.string().optional().describe('Session ID to commit (defaults to MEMFUSE_SESSION_ID or "default")'),
  }),
}, async ({ reason, session_id }) => {
  try {
    const sessionId = session_id || config.sessionId || 'default';

    // POST /sessions/{sessionId}/commit
    const result = await callBackend('POST', `${PATHS.CONSOLIDATE}/${sessionId}/commit`, {
      user_id: config.userId,
      session_id: sessionId,
      reason: reason || 'mcp-manual-commit',
    }, router) as Record<string, unknown>;

    const archiveUri = String(result.archive_uri || '');
    const taskId = result.task_id ? String(result.task_id) : '';

    if (!archiveUri) {
      return { content: [{ type: 'text', text: 'Session committed, but no new content to archive (session was empty or already committed).' }] };
    }

    return {
      content: [{
        type: 'text',
        text: `Session committed successfully.\nArchive: ${archiveUri}${taskId ? `\nConsolidation task: ${taskId}` : ''}\nMemory pipeline will run in background to extract episodes and facts.`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error committing session: ${msg}` }], isError: true };
  }
});

// ─── Tool 5c: session_create ──────────────────────────────────────────────

server.registerTool('session_create', {
  description: 'Create a new MemFuse session. Returns the session_id needed for add_message, resolve_context (with session), store-observation, and commit_session. Use this when you need an explicit session lifecycle rather than the default session.',
  inputSchema: z.object({
    session_id: z.string().optional().describe('Optional custom session ID (auto-generated if not provided)'),
  }),
}, async ({ session_id }) => {
  try {
    // POST /sessions — handler reads session_id from body, identity from AppConfig
    const body: Record<string, unknown> = {};
    if (session_id) body.session_id = session_id;

    const result = await callBackend('POST', '/sessions', body, router) as Record<string, unknown>;
    const newSessionId = String(result.session_id || result.id || 'unknown');

    return {
      content: [{
        type: 'text',
        text: `Session created.\nID: ${newSessionId}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error creating session: ${msg}` }], isError: true };
  }
});

server.registerTool('session_list', {
  description: 'List MemFuse sessions for the configured user. Use this to find existing session IDs before session_get, add_message, or commit_session.',
  inputSchema: z.object({
    limit: z.number().default(20).describe('Maximum number of sessions to return'),
  }),
}, async ({ limit }) => {
  try {
    const params = new URLSearchParams();
    if (limit) params.set('limit', String(limit));
    const query = params.toString();
    const result = await callBackend('GET', `${PATHS.OBSERVE}${query ? `?${query}` : ''}`, null, router) as Record<string, unknown>;
    const sessions = toArray(result.items || result.sessions || result);
    if (sessions.length === 0) {
      return { content: [{ type: 'text', text: 'No sessions found.' }] };
    }
    const lines = sessions.map((session) => {
      const sessionId = String(session.session_id || session.id || '');
      const status = String(session.status || '');
      return `- ${sessionId}${status ? ` (${status})` : ''}`;
    });
    return { content: [{ type: 'text', text: `Sessions (${sessions.length}):\n${lines.join('\n')}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error listing sessions: ${msg}` }], isError: true };
  }
});

server.registerTool('session_get', {
  description: 'Get details for a MemFuse session by ID.',
  inputSchema: z.object({
    session_id: z.string().describe('Session ID to fetch'),
  }),
}, async ({ session_id }) => {
  try {
    const result = await callBackend('GET', `${PATHS.OBSERVE}/${encodeURIComponent(session_id)}`, null, router) as Record<string, unknown>;
    const status = String(result.status || '');
    const turns = result.turns ?? result.message_count ?? result.turn_count ?? '';
    const parts = [`Session: ${String(result.session_id || result.id || session_id)}`];
    if (status) parts.push(`Status: ${status}`);
    if (turns !== '') parts.push(`Turns: ${String(turns)}`);
    return { content: [{ type: 'text', text: parts.join('\n') }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error getting session: ${msg}` }], isError: true };
  }
});

server.registerTool('session_delete', {
  description: 'Delete a MemFuse session by ID. Use with care; this removes the session record and related stored turns.',
  inputSchema: z.object({
    session_id: z.string().describe('Session ID to delete'),
  }),
}, async ({ session_id }) => {
  try {
    await callBackend('DELETE', `${PATHS.OBSERVE}/${encodeURIComponent(session_id)}`, null, router) as Record<string, unknown>;
    return { content: [{ type: 'text', text: `Session deleted.\nID: ${session_id}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error deleting session: ${msg}` }], isError: true };
  }
});

// ─── Tool 5d: add_message ──────────────────────────────────────────────

server.registerTool('add_message', {
  description: 'Add a message (user or assistant turn) to a session. Use this to build the conversation context that MemFuse uses for memory consolidation.',
  inputSchema: z.object({
    session_id: z.string().describe('Session ID to add the message to'),
    role: z.enum(['user', 'assistant']).describe('Role of the message sender'),
    content: z.string().describe('Message content'),
  }),
}, async ({ session_id, role, content }) => {
  try {
    // POST /sessions/{session_id}/messages — body: { role, content }
    const result = await callBackend('POST', `/sessions/${encodeURIComponent(session_id)}/messages`, {
      role, content,
    }, router) as Record<string, unknown>;

    const turnId = String(result.turn_id || result.id || 'added');
    return {
      content: [{
        type: 'text',
        text: `Message added to session ${session_id}.\nTurn ID: ${turnId}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error adding message: ${msg}` }], isError: true };
  }
});

// ─── Tool 5e: create_fact ──────────────────────────────────────────────

server.registerTool('create_fact', {
  description: 'Create a new fact in MemFuse. Facts are stable knowledge entries that persist across sessions. Use this when you discover a definitive piece of knowledge about the user, project, or system.',
  inputSchema: z.object({
    subject: z.string().describe('Subject of the fact (e.g., "project", "user")'),
    predicate: z.string().describe('Predicate/relation (e.g., "uses", "prefers")'),
    display_value: z.string().describe('Value or detail (e.g., "SQLite", "snake_case")'),
    confidence: z.number().min(0).max(1).optional().describe('Confidence score (0.0-1.0, default: 0.9)'),
  }),
}, async ({ subject, predicate, display_value, confidence }) => {
  try {
    // POST /facts — required: id, subject, predicate, display_value
    // id auto-generated (same pattern as CLI)
    const result = await callBackend('POST', PATHS.FACTS, {
      id: `fact_${Date.now()}`,
      subject, predicate, display_value,
      confidence: confidence ?? 0.9,
    }, router) as Record<string, unknown>;

    const factId = String(result.fact_id || result.id || 'created');
    return {
      content: [{
        type: 'text',
        text: `Fact created.\nID: ${factId}\n${subject} ${predicate}: ${display_value}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error creating fact: ${msg}` }], isError: true };
  }
});

// ─── Tool 5f: supersede_fact ──────────────────────────────────────────────

server.registerTool('supersede_fact', {
  description: 'Supersede an old fact with a new one. Creates the replacement fact first, then marks the old fact as superseded by it. The new fact becomes active.',
  inputSchema: z.object({
    old_fact_id: z.string().describe('ID of the fact to supersede'),
    subject: z.string().describe('Subject of the replacement fact'),
    predicate: z.string().describe('Predicate of the replacement fact'),
    display_value: z.string().describe('Value of the replacement fact'),
    confidence: z.number().min(0).max(1).optional().describe('Confidence score for the replacement fact (0.0-1.0, default: 0.9)'),
  }),
}, async ({ old_fact_id, subject, predicate, display_value, confidence }) => {
  try {
    // Step 1: Create the replacement fact (POST /facts)
    const newId = `fact_${Date.now()}`;
    const createResult = await callBackend('POST', PATHS.FACTS, {
      id: newId,
      subject, predicate, display_value,
      confidence: confidence ?? 0.9,
    }, router) as Record<string, unknown>;

    const createdNewId = String(createResult.fact_id || createResult.id || newId);

    // Step 2: Supersede the old fact by the new one (POST /facts/{old}/supersede)
    await callBackend('POST', `${PATHS.FACTS}/${encodeURIComponent(old_fact_id)}/supersede`, {
      new_fact_id: createdNewId,
    }, router) as Record<string, unknown>;

    return {
      content: [{
        type: 'text',
        text: `Fact superseded.\nOld: ${old_fact_id} → New: ${createdNewId}\n${subject} ${predicate}: ${display_value}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error superseding fact: ${msg}` }], isError: true };
  }
});

// ─── Tool 5g: retract_fact ──────────────────────────────────────────────

server.registerTool('retract_fact', {
  description: 'Retract (remove) a fact from active knowledge. Use this when a fact is incorrect or no longer relevant.',
  inputSchema: z.object({
    fact_id: z.string().describe('ID of the fact to retract'),
    reason: z.string().optional().describe('Optional reason for retraction'),
  }),
}, async ({ fact_id, reason }) => {
  try {
    // POST /facts/{fact_id}/retract — body: { reason } (optional)
    const result = await callBackend('POST', `${PATHS.FACTS}/${encodeURIComponent(fact_id)}/retract`, {
      reason: reason || 'retracted via MCP',
    }, router) as Record<string, unknown>;

    return {
      content: [{
        type: 'text',
        text: `Fact retracted.\nID: ${fact_id}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error retracting fact: ${msg}` }], isError: true };
  }
});

// ─── Tool 5h: consolidate ──────────────────────────────────────────────

server.registerTool('consolidate', {
  description: 'Trigger memory consolidation to extract episodes and facts from recent conversations. Use this after committing a session to ensure memories are properly extracted.',
  inputSchema: z.object({
    session_id: z.string().optional().describe('Session ID to consolidate (defaults to current session)'),
    resource_id: z.string().optional().describe('Optional resource ID to consolidate'),
  }),
}, async ({ session_id, resource_id }) => {
  try {
    // POST /v1/memory:consolidate — required: session_id, user_id; optional: resource_id
    const sid = session_id || config.sessionId || 'default';
    const result = await callBackend('POST', '/v1/memory:consolidate', {
      session_id: sid,
      user_id: config.userId,
      ...(resource_id ? { resource_id } : {}),
    }, router) as Record<string, unknown>;

    const taskId = String(result.task_id || result.id || 'started');
    return {
      content: [{
        type: 'text',
        text: `Consolidation triggered.\nTask ID: ${taskId}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error triggering consolidation: ${msg}` }], isError: true };
  }
});

// ─── Tool 5i: extract_facts ──────────────────────────────────────────────

server.registerTool('extract_facts', {
  description: 'Extract facts from text content. Use this when you have raw text that should be analyzed for factual knowledge.',
  inputSchema: z.object({
    texts: z.array(z.string()).describe('Array of text strings to extract facts from'),
  }),
}, async ({ texts }) => {
  try {
    // POST /v1/memory:extract-facts — required: texts, user_id
    const result = await callBackend('POST', '/v1/memory:extract-facts', {
      texts,
      user_id: config.userId,
    }, router) as Record<string, unknown>;

    const facts = Array.isArray(result.facts) ? result.facts : [];
    const factCount = facts.length;
    return {
      content: [{
        type: 'text',
        text: `Fact extraction complete.\nExtracted: ${factCount} fact(s)`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error extracting facts: ${msg}` }], isError: true };
  }
});

// ─── Tool 5c: add_resource ──────────────────────────────────────────────

server.registerTool('add_resource', {
  description: 'Add a resource to the MemFuse knowledge base. Supports local directories, git repositories (local path or remote URL), and inline content. The resource is ingested asynchronously — use task_status to track progress.',
  inputSchema: z.object({
    source_kind: z.enum(['localfs', 'git', 'git_url', 'inline']).describe('Source type: "localfs" for local directory, "git" for local git repo, "git_url" for remote git URL, "inline" for direct content'),
    source_path: z.string().optional().describe('Path to local directory or git repo (required for localfs/git, not for git_url/inline)'),
    url: z.string().optional().describe('Remote git URL (required for git_url, e.g. "https://github.com/org/repo")'),
    logical_name: z.string().optional().describe('Optional logical name for the resource'),
    branch: z.string().optional().describe('Git branch to ingest (default: HEAD)'),
    revision: z.string().optional().describe('Git commit/revision to ingest'),
    file_name: z.string().optional().describe('File name for inline content (required when source_kind=inline)'),
    content: z.string().optional().describe('Content for inline ingestion (required when source_kind=inline)'),
  }),
}, async ({ source_kind, source_path, url, logical_name, branch, revision, file_name, content }) => {
  try {
    let request: Record<string, unknown>;

    if (source_kind === 'inline') {
      if (!file_name || !content) {
        return { content: [{ type: 'text', text: 'Error: file_name and content are required for inline source_kind.', isError: true }] };
      }
      request = { file_name, content, logical_name };
    } else if (source_kind === 'git_url') {
      const gitUrl = url || source_path;
      if (!gitUrl) {
        return { content: [{ type: 'text', text: 'Error: url (or source_path) is required for git_url source_kind.', isError: true }] };
      }
      request = { source_kind: 'git_url', source_path: gitUrl, logical_name, branch, revision };
    } else {
      // localfs or git — source_path is required
      if (!source_path) {
        return { content: [{ type: 'text', text: `Error: source_path is required for source_kind "${source_kind}".`, isError: true }] };
      }
      request = { source_kind, source_path, logical_name, branch, revision };
    }

    const result = await callBackend('POST', PATHS.RESOURCES, request, router) as Record<string, unknown>;

    const resourceId = String(result.resource_id || '');
    const taskKey = String(result.task_key || '');

    return {
      content: [{
        type: 'text',
        text: `Resource added successfully.\nResource ID: ${resourceId}\nTask key: ${taskKey}\nUse \`task_status\` tool to track ingestion progress.`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error adding resource: ${msg}`, isError: true }] };
  }
});

// ─── Tool 5d: add_repo ──────────────────────────────────────────────────

server.registerTool('add_repo', {
  description: 'Add a git repository to the MemFuse knowledge base. Shortcut for add_resource. Supports local repo paths and remote git URLs (GitHub/GitLab ZIP-fast strategy with git clone fallback).',
  inputSchema: z.object({
    repo_path_or_url: z.string().describe('Local path to git repo or remote URL (e.g. "https://github.com/org/repo")'),
    logical_name: z.string().optional().describe('Optional logical name for the repo'),
    branch: z.string().optional().describe('Git branch to ingest (default: main/HEAD)'),
  }),
}, async ({ repo_path_or_url, logical_name, branch }) => {
  try {
    const isUrl = /^(https?:\/\/|git@|ssh:\/\/git@|ssh:\/\/|git:\/\/|ftp:\/\/)/.test(repo_path_or_url);

    const request = isUrl
      ? { source_kind: 'git_url', source_path: repo_path_or_url, logical_name, branch }
      : { source_kind: 'git', source_path: repo_path_or_url, logical_name, branch };

    const result = await callBackend('POST', PATHS.RESOURCES, request, router) as Record<string, unknown>;

    const resourceId = String(result.resource_id || '');
    const taskKey = String(result.task_key || '');

    return {
      content: [{
        type: 'text',
        text: `Repository added successfully.\nResource ID: ${resourceId}\nTask key: ${taskKey}\nUse \`task_status\` tool to track ingestion progress.`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error adding repository: ${msg}`, isError: true }] };
  }
});

// ─── Tool 5e: add_resource_inline ──────────────────────────────────────

server.registerTool('add_resource_inline', {
  description: 'Add inline content directly to the MemFuse knowledge base. Shortcut for add_resource with source_kind=inline. Use this to store code snippets, documents, or any text content without needing a local file or repo.',
  inputSchema: z.object({
    file_name: z.string().describe('Name for the content file (e.g. "auth-design.md", "api-spec.json")'),
    content: z.string().describe('The content to store'),
    logical_name: z.string().optional().describe('Optional logical name for the resource'),
  }),
}, async ({ file_name, content, logical_name }) => {
  try {
    const result = await callBackend('POST', PATHS.RESOURCES, { file_name, content, logical_name }, router) as Record<string, unknown>;

    const resourceId = String(result.resource_id || '');
    const taskKey = String(result.task_key || '');

    return {
      content: [{
        type: 'text',
        text: `Inline resource added successfully.\nFile: ${file_name}\nResource ID: ${resourceId}\nTask key: ${taskKey}\nUse \`task_status\` tool to track ingestion progress.`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error adding inline resource: ${msg}`, isError: true }] };
  }
});

// ─── Tool 5f: task_status ──────────────────────────────────────────────

server.registerTool('task_status', {
  description: 'Check the status of an asynchronous task (e.g., resource ingestion, memory consolidation). Use this to track progress after add_resource, add_repo, or commit_session operations.',
  inputSchema: z.object({
    task_key: z.string().describe('The task key returned by a previous operation'),
  }),
}, async ({ task_key }) => {
  try {
    const result = await callBackend('GET', `/tasks/${encodeURIComponent(task_key)}`, null, router) as Record<string, unknown>;

    const status = String(result.status || 'unknown');
    const processingMode = String(result.processing_mode || '');
    const error = result.error ? extractErrorMessage(result.error) : '';

    if (status === 'completed') {
      return { content: [{ type: 'text', text: `Task completed.\nStatus: ${status}\nProcessing mode: ${processingMode}` }] };
    } else if (status === 'failed') {
      return { content: [{ type: 'text', text: `Task failed.\nStatus: ${status}\nError: ${error}`, isError: true }] };
    } else {
      return { content: [{ type: 'text', text: `Task in progress.\nStatus: ${status}\nProcessing mode: ${processingMode}` }] };
    }
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error checking task status: ${msg}`, isError: true }] };
  }
});

// ─── Tool 6: list_facts ──────────────────────────────────────────────────

server.registerTool('list_facts', {
  description: 'List all active facts for the current user. Returns structured facts with confidence scores.',
  inputSchema: z.object({}),
}, async () => {
  try {
    const result = await callBackend('GET', `${PATHS.FACTS}?user_id=${encodeURIComponent(config.userId)}`, null, router) as Record<string, unknown>;
    const facts = toArray(result.facts);

    if (facts.length === 0) {
      return { content: [{ type: 'text', text: 'No active facts found.' }] };
    }

    const lines = facts.map((f: Record<string, unknown>) => {
      const conf = Number(f.confidence || 0);
      const marker = conf >= 0.9 ? '✓' : conf >= 0.7 ? '~' : '?';
      const subj = String(f.subject || '');
      const pred = String(f.predicate || '');
      const val = String(f.display_value || '');
      return `${marker} **${subj}** → ${pred}: ${val} (confidence: ${conf.toFixed(2)})`;
    });

    return { content: [{ type: 'text', text: `## Active Facts (${facts.length} total)\n\n${lines.join('\n')}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error listing facts: ${msg}` }], isError: true };
  }
});

// ─── Tool 6b: trace_fact ────────────────────────────────────────────────

server.registerTool('trace_fact', {
  description: 'Trace a fact\'s provenance — find the source episode and related assertions. Returns the fact, source episode, and assertions that led to this fact being created. Use when you need to verify where a fact came from.',
  inputSchema: z.object({
    fact_id: z.string().describe('The fact ID to trace'),
  }),
}, async ({ fact_id }) => {
  try {
    const result = await callBackend('GET', `/facts/${encodeURIComponent(fact_id)}/trace`, null, router) as Record<string, unknown>;

    const fact = result.fact as Record<string, unknown> ?? {};
    const sourceEpisodes = toArray(result.source_episodes as unknown);
    const assertions = toArray(result.source_assertions);

    const factLine = `**${String(fact.subject || '')}** → ${String(fact.predicate || '')}: ${String(fact.display_value || '')} (confidence: ${Number(fact.confidence || 0).toFixed(2)}, status: ${String(fact.status || '')})`;

    const episodeLine = sourceEpisodes.length > 0
      ? `\nSource episodes (${sourceEpisodes.length}): ${sourceEpisodes.map((e: Record<string, unknown>) => `${String(e.episode_id || '')} — "${String(e.summary || '')}" (salience: ${Number(e.salience_score || 0).toFixed(2)})`).join('; ')}`
      : '\nSource episodes: none found';

    const assertionLines = assertions.length > 0
      ? `\nSource assertions (${assertions.length}):\n${assertions.map((a: Record<string, unknown>) => `  - ${String(a.assertion_id || '')}: ${String(a.subject || '')} → ${String(a.predicate || '')} (${String(a.operation || '')}, conf: ${Number(a.confidence || 0).toFixed(2)})`).join('\n')}`
      : '\nSource assertions: none found';

    return { content: [{ type: 'text', text: `## Fact Trace\n\n${factLine}${episodeLine}${assertionLines}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error tracing fact: ${msg}` }], isError: true };
  }
});

// ─── Tool 6c: facts_at_time ────────────────────────────────────────────

server.registerTool('facts_at_time', {
  description: 'Query facts that were effective at a specific point in time. Returns facts whose valid_from <= timestamp AND (valid_to IS NULL OR valid_to > timestamp). Useful for "what did the user prefer on April 1?" type queries.',
  inputSchema: z.object({
    at_time: z.string().describe('ISO 8601 timestamp for point-in-time query, e.g. "2026-04-01T00:00:00Z"'),
  }),
}, async ({ at_time }) => {
  try {
    const result = await callBackend('POST', PATHS.CONTEXT_RESOLVE, {
      user_id: config.userId, query: '', token_budget: 1500, strategy: 'precision', at_time,
    }, router) as Record<string, unknown>;

    const sections = result.sections as Record<string, unknown> ?? {};
    const facts = toArray(sections.current_facts);

    if (facts.length === 0) {
      return { content: [{ type: 'text', text: `No facts were effective at ${at_time}.` }] };
    }

    const lines = facts.map((f: Record<string, unknown>) => {
      const pred = String(f.predicate || '');
      const val = String(f.display_value || '');
      const conf = Number(f.confidence || 0).toFixed(2);
      const vf = f.valid_from ? ` (valid since ${String(f.valid_from)})` : '';
      return `- **${pred}**: ${val} [confidence: ${conf}]${vf}`;
    });

    return { content: [{ type: 'text', text: `## Facts effective at ${at_time}\n\n${lines.join('\n')}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error querying facts at time: ${msg}` }], isError: true };
  }
});

// ─── Tool 7: ls ─────────────────────────────────────────────────────────

server.registerTool('ls', {
  description: 'List entries under a MemFuse resource or memory URI. Use after resolve_context/search_memories when a memory hit points to a directory-like mfs:// location.',
  inputSchema: z.object({
    uri: z.string().describe('MemFuse URI to inspect, e.g. mfs://resources/localfs/docs'),
  }),
}, async ({ uri }) => {
  try {
    const result = await callBackend('GET', `${PATHS.LS}?uri=${encodeURIComponent(uri)}`, null, router) as unknown;
    const entries = Array.isArray(result)
      ? result.map((item: Record<string, unknown>) => ({
          name: String(item.name ?? item),
          is_dir: Boolean(item.is_dir ?? false),
        }))
      : [];
    if (entries.length === 0) {
      return { content: [{ type: 'text', text: `No entries found for \`${uri}\`.` }] };
    }
    const baseUri = uri.endsWith('/') ? uri.slice(0, -1) : uri;
    return {
      content: [{
        type: 'text',
        text: `## Listing for \`${uri}\`\n\n${entries.map(item => `- ${item.is_dir ? '📁' : '📄'} ${item.name}\n  \`${baseUri}/${item.name}\``).join('\n')}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error listing URI: ${msg}` }], isError: true };
  }
});

// ─── Tool 8: read ───────────────────────────────────────────────────────

server.registerTool('read', {
  description: 'Read the full text content for a MemFuse URI. Use only after memory retrieval or ls/glob have narrowed you to a specific target.',
  inputSchema: z.object({
    uri: z.string().describe('MemFuse URI to read'),
  }),
}, async ({ uri }) => {
  try {
    const result = await callBackend('GET', `${PATHS.READ}?uri=${encodeURIComponent(uri)}`, null, router) as unknown;
    const text = typeof result === 'string'
      ? result
      : String((result as Record<string, unknown>)?.raw ?? '');
    return { content: [{ type: 'text', text: `## Read \`${uri}\`\n\n${text}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error reading URI: ${msg}` }], isError: true };
  }
});

// ─── Tool 9: abstract ───────────────────────────────────────────────────

server.registerTool('abstract', {
  description: 'Fetch compact L0 summary text for a MemFuse URI. Cheapest way to inspect a pointed-to resource before using read.',
  inputSchema: z.object({
    uri: z.string().describe('MemFuse URI to summarize at the abstract/L0 level'),
  }),
}, async ({ uri }) => {
  try {
    const result = await callBackend('GET', `${PATHS.ABSTRACT}?uri=${encodeURIComponent(uri)}`, null, router) as unknown;
    const text = typeof result === 'string'
      ? result
      : String((result as Record<string, unknown>)?.raw ?? '');
    if (!text) {
      return { content: [{ type: 'text', text: `No L0 abstract available for \`${uri}\`. Try \`overview\` for a deeper summary or \`read\` for full content.` }] };
    }
    return { content: [{ type: 'text', text: `## Abstract for \`${uri}\`\n\n${text}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error fetching abstract: ${msg}` }], isError: true };
  }
});

// ─── Tool 10: overview ──────────────────────────────────────────────────

server.registerTool('overview', {
  description: 'Fetch L1 overview text for a MemFuse URI. Use after abstract when you want more structure but not full raw content.',
  inputSchema: z.object({
    uri: z.string().describe('MemFuse URI to summarize at the overview/L1 level'),
  }),
}, async ({ uri }) => {
  try {
    const result = await callBackend('GET', `${PATHS.OVERVIEW}?uri=${encodeURIComponent(uri)}`, null, router) as unknown;
    const text = typeof result === 'string'
      ? result
      : String((result as Record<string, unknown>)?.raw ?? '');
    if (!text) {
      return { content: [{ type: 'text', text: `No L1 overview available for \`${uri}\`. Try \`read\` for full content or \`abstract\` for a brief summary.` }] };
    }
    return { content: [{ type: 'text', text: `## Overview for \`${uri}\`\n\n${text}` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error fetching overview: ${msg}` }], isError: true };
  }
});

// ─── Tool 11: glob ──────────────────────────────────────────────────────

server.registerTool('glob', {
  description: 'Match file/resource URIs under a MemFuse directory using a glob pattern. Use when memory retrieval points to a broad directory.',
  inputSchema: z.object({
    uri: z.string().describe('Root MemFuse URI to search under'),
    pattern: z.string().describe('Glob pattern, e.g. guides/**/*.md'),
  }),
}, async ({ uri, pattern }) => {
  try {
    const result = await callBackend(
      'GET',
      `${PATHS.GLOB}?uri=${encodeURIComponent(uri)}&pattern=${encodeURIComponent(pattern)}`,
      null,
      router,
    ) as unknown;
    const matches = Array.isArray(result) ? result.map(item => String(item)) : [];
    if (matches.length === 0) {
      return { content: [{ type: 'text', text: `No matches found for pattern \`${pattern}\` under \`${uri}\`.` }] };
    }
    return {
      content: [{
        type: 'text',
        text: `## Glob Matches for \`${pattern}\` under \`${uri}\`\n\n${matches.map(item => `- ${item}`).join('\n')}`,
      }],
    };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error running glob: ${msg}` }], isError: true };
  }
});

// ─── Tool 12: grep ──────────────────────────────────────────────────────

server.registerTool('grep', {
  description: 'Run literal/keyword grep over a narrowed MemFuse scope. Use only after memory retrieval or glob has already reduced the search area.',
  inputSchema: z.object({
    query: z.string().describe('Literal or keyword query to verify'),
    target: z.string().optional().describe('Optional narrowed MemFuse target URI'),
    limit: z.number().optional().describe('Maximum number of results (default 10)'),
  }),
}, async ({ query, target, limit }) => {
  try {
    const params = new URLSearchParams({ query });
    if (target) params.set('target', target);
    if (limit) params.set('limit', String(limit));
    const result = await callBackend('GET', `${PATHS.GREP}?${params.toString()}`, null, router) as Record<string, unknown>;
    return { content: [{ type: 'text', text: formatContextSearchResults('Grep Results', result) }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error running grep: ${msg}` }], isError: true };
  }
});

// ─── Tool 22: cite_memories ────────────────────────────────────────────

server.registerTool('cite_memories', {
  description: 'Record that specific episodes or facts were useful in your response. This increments their recall_count, improving future retrieval ranking. Call this when you reference memories in your answer.',
  inputSchema: z.object({
    episode_ids: z.array(z.string()).optional().describe('Episode IDs that were useful'),
    fact_ids: z.array(z.string()).optional().describe('Fact IDs that were useful'),
  }),
}, async ({ episode_ids, fact_ids }) => {
  try {
    const result = await callBackend('POST', PATHS.CITE, {
      episode_ids: episode_ids ?? [],
      fact_ids: fact_ids ?? [],
    }, router) as Record<string, unknown>;
    const ep = Number(result.cited_episodes || 0);
    const fa = Number(result.cited_facts || 0);
    return { content: [{ type: 'text', text: `Cited ${ep} episode(s) and ${fa} fact(s).` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error citing memories: ${msg}` }], isError: true };
  }
});

// ─── Tool 23: export_memories ──────────────────────────────────────────

server.registerTool('export_memories', {
  description: 'Export all active facts and heuristic rules as editable Markdown. Users can review, edit, and re-import to correct their memory.',
  inputSchema: z.object({}),
}, async () => {
  try {
    const params = new URLSearchParams({ user_id: config.userId });
    const result = await callBackend('GET', `/memories/export?${params.toString()}`, null, router) as Record<string, unknown>;
    return { content: [{ type: 'text', text: String(result.markdown || '') }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error exporting memories: ${msg}` }], isError: true };
  }
});

// ─── Tool 24: import_memories ──────────────────────────────────────────

server.registerTool('import_memories', {
  description: 'Import memories from Markdown (previously exported via export_memories). Updates changed facts and retracts deleted ones.',
  inputSchema: z.object({
    markdown: z.string().describe('The edited Markdown content from export_memories'),
  }),
}, async ({ markdown }) => {
  try {
    const result = await callBackend('POST', '/memories/import', {
      markdown, user_id: config.userId,
    }, router) as Record<string, unknown>;
    const updated = Number(result.updated_facts || 0);
    const retracted = Number(result.retracted_facts || 0);
    const total = Number(result.total_imported || 0);
    return { content: [{ type: 'text', text: `Import complete: ${total} facts parsed, ${updated} updated, ${retracted} retracted.` }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error importing memories: ${msg}` }], isError: true };
  }
});

// ─── Manifest / Canvas / Overlay tools ────────────────────────────────

server.registerTool('get_repo_manifest', {
  description: 'Get the full Repo Knowledge Manifest for a repository. If repo_id is omitted, defaults to the account_id.',
  inputSchema: z.object({
    repo_id: z.string().optional().describe('Unique identifier for the repository (e.g. "github:owner/repo"). Defaults to account_id if omitted.'),
  }),
}, async ({ repo_id }) => {
  try {
    const qp = repo_id ? `repo_id=${encodeURIComponent(repo_id)}` : '';
    const result = await callBackend('GET', `/manifest/get?${qp}`, null, router) as Record<string, unknown>;
    const data = unwrapData(result);
    const repoIdentity = record(data.repo_identity);
    if (!repoIdentity.repo_id) {
      return { content: [{ type: 'text', text: `No manifest found${repo_id ? ` for repo_id: ${repo_id}` : ' (no repo_id specified, tried account_id fallback)'}` }] };
    }
    const text = [
      `**Manifest for ${repoIdentity.repo_id}**`,
      `  resource_uri: ${repoIdentity.resource_uri}`,
      `  default_branch: ${repoIdentity.default_branch}`,
      `  primary_languages: ${JSON.stringify(repoIdentity.primary_languages ?? [])}`,
      `  manifest_yaml_path: ${data.manifest_yaml_path}`,
      `  canvas_indexes: ${toArray(data.canvas_indexes).length}`,
      `  active_overlays: ${toArray(data.active_overlays).length}`,
      `  last_verified_at: ${repoIdentity.last_verified_at}`,
    ].join('\n');
    return { content: [{ type: 'text', text }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error getting manifest: ${msg}` }], isError: true };
  }
});

server.registerTool('query_canvas', {
  description: 'Query the canvas for a repository: returns nodes, edges, active overlays, and conflicts',
  inputSchema: z.object({
    repo_id: z.string().describe('Unique identifier for the repository'),
    component: z.string().optional().describe('Filter by component/module/function name'),
    type: z.enum(['structural', 'contracts', 'status']).optional().describe('Canvas subset to query'),
    node_type: z.string().optional().describe('Optional compatibility filter by node type'),
    status: z.string().optional().describe('Filter overlays by status (proposed, accepted, implemented, merged, abandoned, stale, rejected)'),
  }),
}, async ({ repo_id, component, type, node_type, status }) => {
  try {
    const params = new URLSearchParams({ repo_id });
    if (component) params.set('component', component);
    if (type) params.set('type', type);
    if (node_type) params.set('node_type', node_type);
    if (status) params.set('status', status);
    const result = await callBackend('GET', `/canvas/query?${params.toString()}`, null, router) as Record<string, unknown>;
    const data = unwrapData(result);
    const nodes = toArray(data.nodes as unknown);
    const edges = toArray(data.edges as unknown);
    const overlays = toArray(data.overlays as unknown);
    const conflicts = toArray(data.conflicts as unknown);
    const lines = [
      `**Canvas for ${repo_id}**`,
      `  Nodes: ${nodes.length}`,
      `  Edges: ${edges.length}`,
      `  Overlays: ${overlays.length}`,
      `  Conflicts: ${conflicts.length}`,
    ];
    if (nodes.length > 0) lines.push('\n**Nodes:**');
    for (const n of nodes) {
      lines.push(`  - [${n.node_type}] ${n.name} (${n.id})${n.language ? ` lang:${n.language}` : ''}`);
    }
    if (overlays.length > 0) lines.push('\n**Active Overlays:**');
    for (const o of overlays) {
      lines.push(`  - [${o.overlay_type}] ${o.id} (${o.status})${o.branch ? ` branch:${o.branch}` : ''}`);
    }
    if (conflicts.length > 0) lines.push('\n**Conflicts:**');
    for (const c of conflicts) {
      lines.push(`  - ${c.overlay_a} ↔ ${c.overlay_b} (nodes: ${c.overlap_nodes}, edges: ${c.overlap_edges})`);
    }
    return { content: [{ type: 'text', text: lines.join('\n') }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error querying canvas: ${msg}` }], isError: true };
  }
});

server.registerTool('propose_active_overlay', {
  description: 'Propose a new active overlay (planned change, contract, test, config, or conflict declaration). Agent-authored overlays start in "proposed" status.',
  inputSchema: z.object({
    repo_id: z.string().describe('Repository identifier'),
    overlay_type: z.string().describe('One of: planned_change, planned_contract, conflict_declaration, planned_test, planned_config'),
    content_json: z.unknown().describe('JSON object/string describing the overlay content'),
    affected_nodes: z.array(z.string()).optional().describe('Node IDs this overlay touches'),
    affected_edges: z.array(z.string()).optional().describe('Edge IDs this overlay touches'),
    branch: z.string().optional().describe('Branch name'),
    tracker: z.string().optional().describe('Tracker type (e.g. "github")'),
    tracker_content_id: z.string().describe('Tracker content ID'),
    tracker_project_item_id: z.string().optional().describe('Tracker project item ID'),
    tracker_identifier: z.string().describe('Tracker identifier (e.g. "owner/repo#123")'),
    author: z.string().optional().describe('Author identity; defaults to MEMFUSE_USER_ID'),
  }),
}, async ({ repo_id, overlay_type, content_json, affected_nodes, affected_edges, branch, tracker, tracker_content_id, tracker_project_item_id, tracker_identifier, author }) => {
  try {
    const result = await callBackend('POST', '/overlay/propose', {
      repo_id, overlay_type, content_json,
      affected_nodes, affected_edges, branch,
      tracker, tracker_content_id, tracker_project_item_id, tracker_identifier,
      author: author || config.userId,
    }, router) as Record<string, unknown>;
    const data = unwrapData(result);
    const text = [
      `**Overlay Proposed**`,
      `  overlay_id: ${data.overlay_id}`,
      `  overlay_type: ${overlay_type}`,
      `  status: ${data.status}`,
    ].join('\n');
    return { content: [{ type: 'text', text }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error proposing overlay: ${msg}` }], isError: true };
  }
});

server.registerTool('report_conflict', {
  description: 'Report a conflict between two overlays by checking node/edge overlap',
  inputSchema: z.object({
    repo_id: z.string().describe('Repository identifier'),
    overlay_id_1: z.string().describe('First overlay ID'),
    overlay_id_2: z.string().describe('Second overlay ID'),
    conflict_description: z.string().optional().describe('Optional human-readable description of the conflict'),
  }),
}, async ({ repo_id, overlay_id_1, overlay_id_2, conflict_description }) => {
  try {
    const result = await callBackend('POST', '/overlay/report_conflict', {
      repo_id, overlay_id_1, overlay_id_2, conflict_description,
    }, router) as Record<string, unknown>;
    const data = unwrapData(result);
    const hasConflict = data.has_conflict === true || data.has_conflict === 'true' || data.has_overlap === true;
    const text = [
      `**Conflict Report**`,
      `  overlay_1: ${overlay_id_1}`,
      `  overlay_2: ${overlay_id_2}`,
      `  has_conflict: ${hasConflict}`,
      `  requires_human_review: ${data.requires_human_review}`,
      `  overlap_nodes: ${JSON.stringify(data.overlap_nodes)}`,
      `  overlap_edges: ${JSON.stringify(data.overlap_edges)}`,
    ].join('\n');
    return { content: [{ type: 'text', text }] };
  } catch (error: unknown) {
    const msg = error instanceof Error ? error.message : String(error);
    return { content: [{ type: 'text', text: `Error reporting conflict: ${msg}` }], isError: true };
  }
});

// ─── Format helpers ────────────────────────────────────────────────────

function formatSearchResults(result: Record<string, unknown>): string {
  const results = toArray(result.results);
  const total = Number(result.total || results.length);

  if (results.length === 0) return 'No memories found matching your query.';

  const lines = results.map((r: Record<string, unknown>) => {
    const id = String(r.episode_id || r.id || '');
    const summary = String(r.summary || r.content || '');
    const score = Number(r.score || 0).toFixed(2);
    const salience = Number(r.salience_score || 0).toFixed(2);
    const date = String(r.created_at || '');
    const dateStr = date ? new Date(date).toLocaleDateString() : '';
    return `- **[${dateStr}]** ${summary}\n  ID: \`${id}\` | relevance: ${score} | importance: ${salience}`;
  });

  return `## Search Results (${total} found)\n\n${lines.join('\n\n')}\n\n---\n*Use \`timeline\` or \`get_observations\` with the IDs above for more detail.*`;
}

function formatTimeline(anchorId: string, episodes: Record<string, unknown>[], facts: Record<string, unknown>[]): string {
  const parts = [`## Timeline around \`${anchorId}\``];
  if (episodes.length === 0 && facts.length === 0) {
    parts.push('No context found around this episode.');
    return parts.join('\n');
  }

  if (facts.length > 0) {
    parts.push('\n### Related Facts');
    parts.push(facts.map((f: Record<string, unknown>) => {
      const subj = String(f.subject || '');
      const pred = String(f.predicate || '');
      const val = String(f.display_value || '');
      return `- **${subj}** → ${pred}: ${val}`;
    }).join('\n'));
  }

  if (episodes.length > 0) {
    parts.push('\n### Episode Timeline');
    parts.push(episodes.map((ep: Record<string, unknown>) => {
      const id = String(ep.episode_id || '');
      const summary = String(ep.summary || '');
      const score = Number(ep.score || 0).toFixed(2);
      const date = String(ep.created_at || '');
      const dateStr = date ? new Date(date).toLocaleDateString() : '';
      const marker = id === anchorId ? '▶ **[NOW]**' : `  [${dateStr}]`;
      return `${marker} ${summary} (relevance: ${score})\n  ID: \`${id}\``;
    }).join('\n\n'));
    parts.push('\n---\n*Use `get_observations` with specific IDs for full details.*');
  }

  return parts.join('\n');
}

function formatObservations(details: Record<string, unknown>[]): string {
  if (details.length === 0) return 'No observation details retrieved.';

  const parts = ['## Observation Details'];
  details.forEach((d: Record<string, unknown>, i: number) => {
    if (d.error) {
      parts.push(`\n### (${i + 1}) ID: \`${d.id}\` — ${extractErrorMessage(d.error)}`);
      return;
    }

    const id = String(d.episode_id || d.id || `detail-${i}`);
    const summary = String(d.summary || d.content || '');
    const facts = toArray(d.facts);
    const turns = toArray(d.turns || d.source_turns);
    const created = String(d.created_at || '');

    parts.push(`\n### (${i + 1}) Episode: \`${id}\``);
    if (created) parts.push(`Created: ${new Date(created).toLocaleString()}`);
    if (summary) parts.push(`\n**Summary:** ${summary}`);
    if (facts.length > 0) {
      parts.push('\n**Facts:**');
      facts.forEach((f: Record<string, unknown>) => {
        parts.push(`- ${String(f.subject || '')} → ${String(f.predicate || '')}: ${String(f.display_value || '')}`);
      });
    }
    if (turns.length > 0 && typeof turns[0] === 'object') {
      parts.push('\n**Source Turns:**');
      turns.forEach((t: Record<string, unknown>) => {
        const role = String(t.role || '');
        const content = String(t.content || '');
        if (role && content) {
          const truncated = content.length > 300 ? content.substring(0, 300) + '...' : content;
          parts.push(`[${role}] ${truncated}`);
        }
      });
    } else if (turns.length > 0) {
      parts.push(`\n**Source Turn IDs:** ${turns.join(', ')}`);
    }
  });

  return parts.join('\n');
}

function buildContent(toolName: string, toolInput?: string, toolOutput?: string): string {
  const parts = [`Tool: ${toolName}`];
  if (toolInput) parts.push(`Input:\n${toolInput}`);
  if (toolOutput) parts.push(`Output:\n${toolOutput}`);
  return parts.join('\n');
}

function formatContextSearchResults(title: string, result: Record<string, unknown>): string {
  const buckets = [
    ['resources', toArray(result.resources)],
    ['memories', toArray(result.memories)],
    ['skills', toArray(result.skills)],
  ] as const;
  const lines: string[] = [`## ${title}`];

  for (const [label, items] of buckets) {
    if (items.length === 0) continue;
    lines.push(`\n### ${capitalize(label)}`);
    for (const item of items) {
      const uri = String(item.uri || '');
      const summary = String(item.summary || item.excerpt || item.content || item.match_reason || '');
      const score = item.score != null ? ` (score: ${Number(item.score).toFixed(2)})` : '';
      lines.push(`- \`${uri}\`${score}${summary ? `\n  ${summary}` : ''}`);
    }
  }

  if (lines.length === 1) {
    return `${lines[0]}\n\nNo matches found.`;
  }
  return lines.join('\n');
}

function capitalize(value: string): string {
  if (!value) return value;
  return value[0].toUpperCase() + value.slice(1);
}

// ─── Start server ──────────────────────────────────────────────────────

export async function startServer(): Promise<void> {
  startParentHeartbeat();
  const transport = new StdioServerTransport();
  await server.connect(transport);
  console.error('MemFuse MCP server started (v0.1.0)');
  console.error(`  USER_ID: ${config.userId}`);
  console.error(`  SESSION_ID: ${config.sessionId || '(auto)'}`);
  console.error(`  SERVER_URL: ${config.serverUrl}`);
}
