import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum,
} from '../types.js';
import {
  formatWorkspaceWrite,
} from '../output.js';

export function registerWorkspaceCommands(register: RegisterFn): void {
  register('mkdir', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const result = await call('POST', '/mkdir', { uri }, args) as Record<string, unknown>;
    output(formatWorkspaceWrite('mkdir', uri, result, args.mode), args);
  });

  register('write', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const content = optStr(args, 'content');
    if (!content) {
      console.error('Error: write requires --content.');
      process.exit(1);
    }
    const result = await call('POST', '/write', { uri, content }, args) as Record<string, unknown>;
    output(formatWorkspaceWrite('write', uri, result, args.mode), args);
  });

  register('mv', async (args) => {
    const from = requirePositional(args, 'from', 0);
    const to = requirePositional(args, 'to', 1);
    const result = await call('POST', '/mv', { from_uri: from, to_uri: to }, args) as Record<string, unknown>;
    output(formatWorkspaceWrite('mv', `${from} → ${to}`, result, args.mode), args);
  });

  register('rm', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const result = await call('DELETE', `/rm?uri=${encodeURIComponent(uri)}`, null, args) as Record<string, unknown>;
    output(formatWorkspaceWrite('rm', uri, result, args.mode), args);
  });

  // ── Workspace extended ops ──────────────────────────────────────
  register('tree', async (args) => {
    const uri = optStr(args, 'uri') ?? 'mfs://resources/';
    const depth = optNum(args, 'depth');
    const params = new URLSearchParams({ uri });
    if (depth !== undefined) params.set('depth', String(depth));

    const result = await call('GET', `/tree?${params.toString()}`, null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    // Server returns plain text tree wrapped as { raw: "..." }
    if (typeof result === 'string') { output(result, args); return; }
    if (typeof result.raw === 'string') { output(result.raw, args); return; }
    // Fallback: structured entries
    const entries: Record<string, unknown>[] = Array.isArray(result.entries) ? result.entries as Record<string, unknown>[] : [];
    if (entries.length === 0) { output(`Tree: ${uri} (empty)`, args); return; }
    const lines = entries.map((e: Record<string, unknown>) => {
      const prefix = e.is_dir ? '[DIR] ' : '      ';
      return `${prefix}${String(e.name || e.uri || '')}`;
    });
    output(`Tree: ${uri}\n${lines.join('\n')}`, args);
  });

  register('stat', async (args) => {
    const uri = optStr(args, 'uri') ?? 'mfs://resources/';
    const result = await call('GET', `/stat?uri=${encodeURIComponent(uri)}`, null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const lines = Object.entries(result).map(([k, v]) => `${k}: ${typeof v === 'object' ? JSON.stringify(v) : v}`);
    output(`Stat: ${uri}\n${lines.join('\n')}`, args);
  });

  register('rebuild', async (args) => {
    const result = await call('GET', '/rebuild', null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(`Rebuild triggered: ${String(result.status ?? result.message ?? 'ok')}`, args);
  });

  register('refresh', async (args) => {
    const result = await call('POST', '/refresh', null, args) as Record<string, unknown>;
    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(`Refresh triggered: ${String(result.status ?? result.message ?? 'ok')}`, args);
  });
}