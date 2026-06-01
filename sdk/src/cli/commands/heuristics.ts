import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optNum, optStr, splitComma,
} from '../types.js';
import {
  formatSimulateReaction, formatHeuristicsL0, formatConfirmRule,
} from '../output.js';

export function registerHeuristicCommands(register: RegisterFn): void {
  register('simulate-reaction', async (args) => {
    const scenario = requirePositional(args, 'scenario', 0);
    const tags = splitComma(optStr(args, 'tags'));

    const result = await call('POST', '/heuristics/simulate-reaction', {
      scenario, tags, user_id: args.config.userId,
    }, args) as Record<string, unknown>;

    output(formatSimulateReaction(result, args.mode), args);
  });

  register('heuristics-l0', async (args) => {
    const maxRules = optNum(args, 'max-rules') ?? 5;

    const result = await call('POST', '/heuristics/l0-confirmed', {
      max_rules: maxRules, user_id: args.config.userId,
    }, args);

    output(formatHeuristicsL0(result, args.mode), args);
  });

  register('confirm-rule', async (args) => {
    const ruleId = requirePositional(args, 'rule-id', 0);
    const result = await call('POST', `/heuristics/rules/${encodeURIComponent(ruleId)}/confirm`, {
      user_id: args.config.userId,
    }, args) as Record<string, unknown>;
    output(formatConfirmRule(result, ruleId, args.mode), args);
  });
}