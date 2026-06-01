import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum, splitComma,
} from '../types.js';
import {
  formatCodeSymbolsList, formatCodeSymbolsSearch, formatCodeSymbolsCreated,
  formatCodeSymbolsDeleted,
} from '../output.js';

export function registerCodeSymbolCommands(register: RegisterFn): void {
  register('code-symbols-list', async (args) => {
    const projectionViewId = optStr(args, 'projection-view-id');
    const canonicalUri = optStr(args, 'canonical-uri');
    const params = new URLSearchParams();
    if (projectionViewId) params.set('projection_view_id', projectionViewId);
    if (canonicalUri) params.set('canonical_uri', canonicalUri);
    const qs = params.toString() ? `?${params.toString()}` : '';

    const result = await call('GET', `/code_symbols${qs}`, null, args);
    output(formatCodeSymbolsList(result, args.mode), args);
  });

  register('code-symbols-search', async (args) => {
    const query = requirePositional(args, 'query', 0);
    const projectionViewId = optStr(args, 'projection-view-id');
    if (!projectionViewId) {
      console.error('Error: code-symbols-search requires --projection-view-id.');
      process.exit(1);
    }
    const params = new URLSearchParams({ q: query, projection_view_id: projectionViewId });

    const result = await call('GET', `/code_symbols/search?${params.toString()}`, null, args) as Record<string, unknown>;
    output(formatCodeSymbolsSearch(result, args.mode), args);
  });

  register('code-symbols-create', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const projectionViewId = optStr(args, 'projection-view-id');
    if (!projectionViewId) {
      console.error('Error: code-symbols-create requires --projection-view-id.');
      process.exit(1);
    }
    const symbolNames = splitComma(optStr(args, 'symbols'));
    const symbolTypes = splitComma(optStr(args, 'symbol-types'));
    const signatures = splitComma(optStr(args, 'signatures'));
    const docstrings = splitComma(optStr(args, 'docstrings'));

    if (symbolNames.length === 0) {
      console.error('Error: code-symbols-create requires --symbols (comma-separated symbol names).');
      process.exit(1);
    }

    const symbols = symbolNames.map((name, i) => ({
      id: `sym_${Date.now()}_${i}`,
      projection_view_id: projectionViewId,
      canonical_uri: uri,
      symbol_type: symbolTypes[i] ?? 'function',
      symbol_name: name,
      ...(signatures[i] ? { signature: signatures[i] } : {}),
      ...(docstrings[i] ? { docstring: docstrings[i] } : {}),
    }));

    const result = await call('POST', '/code_symbols', {
      symbols,
    }, args) as Record<string, unknown>;

    output(formatCodeSymbolsCreated(result, args.mode), args);
  });

  register('code-symbols-delete', async (args) => {
    const viewId = requirePositional(args, 'view-id', 0);
    const result = await call('DELETE', `/code_symbols/${encodeURIComponent(viewId)}`, null, args) as Record<string, unknown>;
    output(formatCodeSymbolsDeleted(result, args.mode), args);
  });
}