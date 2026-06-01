import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optNum, optStr, sessionId, splitComma, buildContent,
} from '../types.js';
import {
  formatResolveContext, formatSearchResults, formatTimeline,
  formatObservations, formatObservationStored, formatSessionCommitted,
  formatFacts, toArray,
} from '../output.js';

export function registerCoreCommands(register: RegisterFn): void {
  // ── Shared resolve helper ────────────────────────────────────────
  async function resolveAndOutput(args: CliArgs): Promise<void> {
    const query = requirePositional(args, 'query', 0);
    const budget = optNum(args, 'budget') ?? 1500;
    const strategy = optStr(args, 'strategy');
    const atTime = optStr(args, 'at-time');
    const resourceId = optStr(args, 'resource-id');

    const result = await call('POST', '/context/resolve', {
      query,
      session_id: sessionId(args),
      token_budget: budget,
      user_id: args.config.userId,
      ...(strategy ? { strategy } : {}),
      ...(atTime ? { at_time: atTime } : {}),
      ...(resourceId ? { resource_id: resourceId } : {}),
    }, args) as Record<string, unknown>;

    output(formatResolveContext(result, args.mode), args);
  }

  register('resolve-context', resolveAndOutput);
  register('inject-context', resolveAndOutput);

  register('search', async (args) => {
    const query = requirePositional(args, 'query', 0);
    const limit = optNum(args, 'limit') ?? 10;
    const strategy = optStr(args, 'strategy');
    const topK = optNum(args, 'top-k');
    const threadId = optStr(args, 'thread-id');

    const result = await call('POST', '/v1/memory:search', {
      query,
      ...(topK !== undefined ? { top_k: topK } : { limit }),
      user_id: args.config.userId,
      ...(args.sessionExplicit ? { session_id: sessionId(args) } : {}),
      ...(strategy ? { strategy } : {}),
      ...(threadId ? { thread_id: threadId } : {}),
    }, args) as Record<string, unknown>;

    output(formatSearchResults(result, args.mode), args);
  });

  register('timeline', async (args) => {
    const episodeId = requirePositional(args, 'episode-id', 0);
    const direction = optStr(args, 'direction') ?? 'both';
    const radius = optNum(args, 'radius') ?? 3;

    const params = new URLSearchParams({ direction, radius: String(radius) });
    const result = await call('GET',
      `/episodes/${encodeURIComponent(episodeId)}/timeline?${params.toString()}`,
      null, args) as Record<string, unknown>;

    const episodes = toArray(result.episodes);
    const facts = toArray(result.current_facts);
    output(formatTimeline(episodeId, episodes, facts, args.mode), args);
  });

  register('get-observations', async (args) => {
    const idsStr = requirePositional(args, 'episode-ids', 0);
    const ids = splitComma(idsStr);

    if (ids.length === 0) {
      console.error('Error: No episode IDs provided. Pass comma-separated IDs.');
      process.exit(1);
    }

    const details = await Promise.all(ids.map(async (id) => {
      try {
        return await call('GET', `/episodes/${encodeURIComponent(id)}`, null, args) as Record<string, unknown>;
      } catch (_) {
        return { id, error: 'unavailable' };
      }
    }));

    output(formatObservations(details, args.mode), args);
  });

  register('store-observation', async (args) => {
    const content = args.positional[0];
    const toolName = optStr(args, 'type') ?? 'Observation';
    const toolInput = optStr(args, 'input');
    const toolOutput = optStr(args, 'output');
    const sourceTrust = optStr(args, 'source-trust');
    const metadata = optStr(args, 'metadata');

    const obsContent = content || buildContent(toolName, toolInput, toolOutput);

    if (!obsContent) {
      console.error('Error: store-observation requires content or --type/--input/--output flags.');
      process.exit(1);
    }

    let metadataJson: unknown = undefined;
    if (metadata) {
      try { metadataJson = JSON.parse(metadata); } catch { console.error('Error: --metadata must be valid JSON.'); process.exit(1); }
    }

    const result = await call('POST', `/sessions/${sessionId(args)}/observations`, {
      tool_name: toolName,
      tool_input: toolInput ?? '',
      tool_output: toolOutput ?? '',
      content: obsContent,
      platform: 'cli',
      ...(sourceTrust ? { source_trust: sourceTrust } : {}),
      ...(metadataJson ? { metadata: metadataJson } : {}),
    }, args) as Record<string, unknown>;

    output(formatObservationStored(result, args.mode), args);
  });

  register('commit-session', async (args) => {
    const reason = optStr(args, 'reason') ?? 'cli-manual-commit';

    const result = await call('POST', `/sessions/${sessionId(args)}/commit`, {
      user_id: args.config.userId,
      reason,
    }, args) as Record<string, unknown>;

    output(formatSessionCommitted(result, args.mode), args);
  });

  register('list-facts', async (args) => {
    const result = await call('GET',
      `/facts?user_id=${encodeURIComponent(args.config.userId)}`,
      null, args) as Record<string, unknown>;

    const facts = toArray(result.items || result.facts);
    output(formatFacts(facts, args.mode, result), args);
  });
}
