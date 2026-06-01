import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum,
} from '../types.js';
import {
  formatRelationLink, formatRelationUnlink, formatRelationList,
} from '../output.js';

export function registerRelationCommands(register: RegisterFn): void {
  register('link', async (args) => {
    const from = requirePositional(args, 'from', 0);
    const to = requirePositional(args, 'to', 1);
    const relationType = optStr(args, 'relation-type') ?? 'related';

    const result = await call('POST', '/relations', {
      from_uri: from, to_uri: to, relation_type: relationType,
    }, args) as Record<string, unknown>;

    output(formatRelationLink(result, args.mode), args);
  });

  register('unlink', async (args) => {
    const from = requirePositional(args, 'from', 0);
    const to = requirePositional(args, 'to', 1);
    const relationType = optStr(args, 'relation-type') ?? 'related';

    const params = new URLSearchParams({
      from_uri: from, to_uri: to, relation_type: relationType,
    });

    const result = await call('DELETE', `/relations?${params.toString()}`, null, args) as Record<string, unknown>;

    output(formatRelationUnlink(result, args.mode), args);
  });

  register('relations', async (args) => {
    const uri = requirePositional(args, 'uri', 0);
    const limit = optNum(args, 'limit') ?? 20;
    const params = new URLSearchParams({ uri, limit: String(limit) });

    const result = await call('GET', `/relations?${params.toString()}`, null, args) as Record<string, unknown>;
    output(formatRelationList(result, args.mode), args);
  });
}