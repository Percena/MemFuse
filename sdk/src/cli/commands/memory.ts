import {
  CliArgs, RegisterFn, call, output, optStr, optNum, requirePositional, splitComma, sessionId,
} from '../types.js';
import {
  formatCiteMemories, formatExportMemories, formatImportMemories,
} from '../output.js';

export function registerMemoryCommands(register: RegisterFn): void {
  register('cite-memories', async (args) => {
    const episodeIds = splitComma(optStr(args, 'episode-ids'));
    const factIds = splitComma(optStr(args, 'fact-ids'));

    const result = await call('POST', '/memories/cite', {
      episode_ids: episodeIds, fact_ids: factIds,
    }, args) as Record<string, unknown>;

    output(formatCiteMemories(result, args.mode), args);
  });

  register('export-memories', async (args) => {
    const result = await call('GET',
      `/memories/export?user_id=${encodeURIComponent(args.config.userId)}`,
      null, args) as Record<string, unknown>;

    output(formatExportMemories(result, args.mode), args);
  });

  register('import-memories', async (args) => {
    const filePath = optStr(args, 'file');

    let markdown: string;
    if (filePath) {
      const fs = await import('node:fs/promises');
      markdown = await fs.readFile(filePath, 'utf-8');
    } else {
      const chunks: Buffer[] = [];
      for await (const chunk of process.stdin) {
        chunks.push(chunk as Buffer);
      }
      markdown = Buffer.concat(chunks).toString('utf-8');
      if (!markdown.trim()) {
        console.error('Error: No input provided. Use --file <path> or pipe content via stdin.');
        process.exit(1);
      }
    }

    const result = await call('POST', '/memories/import', {
      markdown, user_id: args.config.userId,
    }, args) as Record<string, unknown>;

    output(formatImportMemories(result, args.mode), args);
  });

  // ── Memory pipeline ─────────────────────────────────────────────
  register('consolidate', async (args) => {
    const consolidateSessionId = optStr(args, 'session-id') ?? sessionId(args);
    const resourceId = optStr(args, 'resource-id');

    const result = await call('POST', '/v1/memory:consolidate', {
      session_id: consolidateSessionId,
      user_id: args.config.userId,
      ...(resourceId ? { resource_id: resourceId } : {}),
    }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const factsCount = Number(result.facts_created ?? result.new_facts ?? 0);
    output(`Consolidated session ${consolidateSessionId} (${factsCount} facts extracted)`, args);
  });

  register('extract-facts', async (args) => {
    const texts = args.positional.length > 0
      ? args.positional
      : splitComma(optStr(args, 'texts'));
    if (texts.length === 0) {
      console.error('Error: extract-facts requires texts as positional args or --texts comma-separated.');
      process.exit(1);
    }

    const result = await call('POST', '/v1/memory:extract-facts', {
      texts, user_id: args.config.userId,
    }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const assertions = Array.isArray(result.assertions) ? result.assertions : [];
    output(`Extracted ${assertions.length} assertions from ${texts.length} texts`, args);
  });

  register('archive', async (args) => {
    const hotnessThreshold = optNum(args, 'hotness-threshold');
    const minAgeDays = optNum(args, 'min-age-days');

    const result = await call('POST', '/v1/memory:archive', {
      ...(hotnessThreshold !== undefined ? { hotness_threshold: hotnessThreshold } : {}),
      ...(minAgeDays !== undefined ? { min_age_days: minAgeDays } : {}),
    }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const archived = Number(result.archived_episodes ?? 0);
    output(`Archived ${archived} cold episodes`, args);
  });

  register('eval-recall', async (args) => {
    const query = requirePositional(args, 'query', 0);
    const expectedFacts = splitComma(optStr(args, 'expected-facts'));
    const k = optNum(args, 'k') ?? 10;

    if (expectedFacts.length === 0) {
      console.error('Error: eval-recall requires --expected-facts (comma-separated fact IDs or texts).');
      process.exit(1);
    }

    const result = await call('POST', '/v1/eval/recall', {
      query, expected_facts: expectedFacts, k,
    }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const recallAtK = Number(result.recall_at_k ?? 0);
    const matched = Number(result.matched_count ?? 0);
    const expected = Number(result.expected_count ?? expectedFacts.length);
    output(`Recall@${k}: ${recallAtK.toFixed(2)} (${matched}/${expected} matched)`, args);
  });
}
