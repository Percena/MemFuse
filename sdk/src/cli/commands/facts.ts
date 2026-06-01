import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum,
} from '../types.js';
import {
  formatFacts, formatFactCreated, formatFactSuperseded, formatFactRetracted,
  formatFactTrace,
} from '../output.js';

export function registerFactCommands(register: RegisterFn): void {
  register('create-fact', async (args) => {
    const subject = requirePositional(args, 'subject', 0);
    const predicate = requirePositional(args, 'predicate', 1);
    const value = requirePositional(args, 'value', 2);
    const id = optStr(args, 'id') ?? `fact_${Date.now()}`;
    const confidence = optNum(args, 'confidence');
    const valueType = optStr(args, 'value-type');
    const agentId = optStr(args, 'agent-id');
    const sourceAssertionId = optStr(args, 'source-assertion-id');
    const sourceEpisodeIds = optStr(args, 'source-episode-ids');

    const result = await call('POST', '/facts', {
      id, subject, predicate, display_value: value,
      ...(confidence !== undefined ? { confidence } : {}),
      ...(valueType ? { value_type: valueType } : {}),
      ...(agentId ? { agent_id: agentId } : {}),
      ...(sourceAssertionId ? { source_assertion_id: sourceAssertionId } : {}),
      ...(sourceEpisodeIds ? { source_episode_ids_json: sourceEpisodeIds } : {}),
    }, args) as Record<string, unknown>;

    output(formatFactCreated(result, args.mode), args);
  });

  register('supersede-fact', async (args) => {
    const factId = requirePositional(args, 'fact-id', 0);
    const newFactId = optStr(args, 'new-fact-id') ?? `fact_${Date.now()}`;

    const result = await call('POST', `/facts/${encodeURIComponent(factId)}/supersede`, {
      new_fact_id: newFactId,
    }, args) as Record<string, unknown>;

    output(formatFactSuperseded(result, args.mode), args);
  });

  register('retract-fact', async (args) => {
    const factId = requirePositional(args, 'fact-id', 0);

    const result = await call('POST', `/facts/${encodeURIComponent(factId)}/retract`, {}, args) as Record<string, unknown>;

    output(formatFactRetracted(result, args.mode), args);
  });

  register('trace-fact', async (args) => {
    const factId = requirePositional(args, 'fact-id', 0);

    const result = await call('GET',
      `/facts/${encodeURIComponent(factId)}/trace`,
      null, args) as Record<string, unknown>;

    output(formatFactTrace(factId, result, args.mode), args);
  });
}