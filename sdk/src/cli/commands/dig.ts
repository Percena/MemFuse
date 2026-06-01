import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum,
} from '../types.js';
import {
  formatLs, formatText, formatGlob, formatContextSearchResults, toArray,
} from '../output.js';

export function registerDigCommands(register: RegisterFn): void {
  register('ls', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const result = await call('GET', `/ls?uri=${encodeURIComponent(uri)}`, null, args) as unknown;

    const entries = Array.isArray(result)
      ? result.map((item: Record<string, unknown>) => ({
          name: String(item.name ?? item),
          is_dir: Boolean(item.is_dir ?? false),
        }))
      : [];

    output(formatLs(uri, entries, args.mode), args);
  });

  register('read', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const result = await call('GET', `/read?uri=${encodeURIComponent(uri)}`, null, args) as unknown;
    if (args.mode === 'json') {
      output(JSON.stringify(result), args);
    } else {
      const text = typeof result === 'string'
        ? result
        : String((result as Record<string, unknown>)?.raw ?? '');
      output(formatText('Read', uri, text, args.mode), args);
    }
  });

  register('abstract', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const result = await call('GET', `/abstract?uri=${encodeURIComponent(uri)}`, null, args) as unknown;
    if (args.mode === 'json') {
      output(JSON.stringify(result), args);
    } else {
      const text = typeof result === 'string'
        ? result
        : String((result as Record<string, unknown>)?.raw ?? '');
      output(formatText('Abstract', uri, text, args.mode), args);
    }
  });

  register('overview', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const result = await call('GET', `/overview?uri=${encodeURIComponent(uri)}`, null, args) as unknown;
    if (args.mode === 'json') {
      output(JSON.stringify(result), args);
    } else {
      const text = typeof result === 'string'
        ? result
        : String((result as Record<string, unknown>)?.raw ?? '');
      output(formatText('Overview', uri, text, args.mode), args);
    }
  });

  register('glob', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const pattern = requirePositional(args, 'pattern', 1);
    const result = await call('GET',
      `/glob?uri=${encodeURIComponent(uri)}&pattern=${encodeURIComponent(pattern)}`,
      null, args) as unknown;

    const matches = Array.isArray(result) ? result.map(item => String(item)) : [];
    output(formatGlob(uri, pattern, matches, args.mode), args);
  });

  register('grep', async (args) => {
    const query = requirePositional(args, 'query', 0);
    const params = new URLSearchParams({ query });
    const target = optStr(args, 'target');
    const limit = optNum(args, 'limit');
    if (target) params.set('target', target);
    if (limit !== undefined) params.set('limit', String(limit));

    const result = await call('GET', `/grep?${params.toString()}`, null, args) as Record<string, unknown>;
    output(formatContextSearchResults('Grep Results', result, args.mode), args);
  });

  register('find', async (args) => {
    const query = requirePositional(args, 'query', 0);
    const params = new URLSearchParams({ query });
    const target = optStr(args, 'target');
    if (target) params.set('target', target);

    const result = await call('GET', `/find?${params.toString()}`, null, args) as Record<string, unknown>;
    output(formatContextSearchResults('Find Results', result, args.mode), args);
  });

  register('search-context', async (args) => {
    const query = requirePositional(args, 'query', 0);
    const params = new URLSearchParams({ query });
    const target = optStr(args, 'target');
    const sessionContext = optStr(args, 'session-context');
    if (target) params.set('target', target);
    if (sessionContext) params.set('session_context', sessionContext);

    const result = await call('GET', `/search?${params.toString()}`, null, args) as Record<string, unknown>;
    output(formatContextSearchResults('Context Search', result, args.mode), args);
  });
}