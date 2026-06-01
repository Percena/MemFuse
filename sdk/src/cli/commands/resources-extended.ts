import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum, splitComma,
} from '../types.js';
import {
  formatResourceAdded, formatResourceList, formatResourceExport, formatResourceImportResult,
  formatTaskStatus, formatTaskList, toArray,
} from '../output.js';

export function registerResourceExtendedCommands(register: RegisterFn): void {
  register('resource-export', async (args) => {
    const resourceId = requirePositional(args, 'resource-id', 0);
    const outputPath = optStr(args, 'output-path');
    if (!outputPath) {
      console.error('Error: resource-export requires --output-path.');
      process.exit(1);
    }

    const result = await call('POST',
      `/resources/${encodeURIComponent(resourceId)}/export`,
      { output_path: outputPath }, args) as Record<string, unknown>;

    output(formatResourceExport(result, args.mode), args);
  });

  register('resource-import', async (args) => {
    const packPath = requirePositional(args, 'pack-path', 0);
    const name = optStr(args, 'name');

    const result = await call('POST', '/resources/import', {
      pack_path: packPath,
      ...(name ? { logical_name: name } : {}),
    }, args) as Record<string, unknown>;

    output(formatResourceImportResult(result, args.mode), args);
  });

  register('add-batch', async (args) => {
    const pathsStr = optStr(args, 'paths');
    const paths = pathsStr ? splitComma(pathsStr) : args.positional;
    const sourceKind = optStr(args, 'source-kind');
    if (paths.length === 0) {
      console.error('Error: add-batch requires --paths (comma-separated) or positional paths.');
      process.exit(1);
    }

    const resources = paths.map((p: string) => {
      const isUrl = /^(https?:\/\/|git@|ssh:\/\/)/.test(p);
      const kind = sourceKind ?? (isUrl ? 'git_url' : 'localfs');
      return { source_kind: kind, source_path: p };
    });

    const result = await call('POST', '/resources/batch', {
      resources,
    }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const items = toArray(result.results || result.resources || result);
    output(`Batch added ${items.length} resources`, args);
  });

  register('snapshots', async (args) => {
    const limit = optNum(args, 'limit');
    const params = new URLSearchParams();
    if (limit !== undefined) params.set('limit', String(limit));
    const qs = params.toString() ? `?${params.toString()}` : '';

    const result = await call('GET', `/snapshots${qs}`, null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const items = toArray(result.items || result.snapshots || result);
    if (items.length === 0) { output('No snapshots found.', args); return; }
    const lines = items.map((s: Record<string, unknown>) => `${String(s.id || s.snapshot_id || '')} ${String(s.created_at || '')}`);
    output(`Snapshots (${items.length}):\n${lines.join('\n')}`, args);
  });

  register('audit', async (args) => {
    const limit = optNum(args, 'limit');
    const params = new URLSearchParams();
    if (limit !== undefined) params.set('limit', String(limit));
    const qs = params.toString() ? `?${params.toString()}` : '';

    const result = await call('GET', `/audit${qs}`, null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const items = toArray(result.items || result.audit || result.entries || result);
    if (items.length === 0) { output('No audit entries found.', args); return; }
    const lines = items.map((a: Record<string, unknown>) => `${String(a.timestamp || a.created_at || '')} ${String(a.action || '')} ${String(a.resource_id || a.uri || '')}`);
    output(`Audit (${items.length}):\n${lines.join('\n')}`, args);
  });

  register('evict-tasks', async (args) => {
    const result = await call('POST', '/tasks/evict', null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const evicted = Number(result.evicted ?? result.count ?? 0);
    output(`Evicted ${evicted} stale tasks`, args);
  });

  register('wait-task', async (args) => {
    const taskId = requirePositional(args, 'task-id', 0);
    const timeoutMs = optNum(args, 'timeout-ms') ?? 30000;
    const pollMs = optNum(args, 'poll-ms') ?? 1000;
    const params = new URLSearchParams({ timeout_ms: String(timeoutMs), poll_ms: String(pollMs) });

    const result = await call('GET', `/tasks/${encodeURIComponent(taskId)}/wait?${params.toString()}`, null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(formatTaskStatus(result, args.mode), args);
  });
}
