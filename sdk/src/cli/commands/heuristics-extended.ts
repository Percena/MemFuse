import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optStr, optNum, splitComma,
} from '../types.js';
import {
  formatHeuristicRuleCreated, formatHeuristicRuleList, formatHeuristicRuleDetail,
  formatHeuristicRulePromoted, formatHeuristicInstanceCreated,
  formatHeuristicInstanceList, formatHeuristicInstanceDetail,
  formatHeuristicRetrieve,
} from '../output.js';

export function registerHeuristicExtendedCommands(register: RegisterFn): void {
  // ── Rule CRUD ────────────────────────────────────────────────────
  register('create-rule', async (args) => {
    const ruleText = requirePositional(args, 'rule-text', 0);
    const tags = splitComma(optStr(args, 'tags'));
    const counterExamples = splitComma(optStr(args, 'counter-examples'));
    const lifecycleStage = optStr(args, 'lifecycle-stage');

    const result = await call('POST', '/heuristics/rules', {
      rule_text: ruleText, tags,
      ...(counterExamples.length > 0 ? { counter_examples: counterExamples } : {}),
      ...(lifecycleStage ? { lifecycle_stage: lifecycleStage } : {}),
      user_id: args.config.userId,
    }, args) as Record<string, unknown>;

    output(formatHeuristicRuleCreated(result, args.mode), args);
  });

  register('list-rules', async (args) => {
    const lifecycleStage = optStr(args, 'lifecycle-stage');
    const params = new URLSearchParams();
    if (lifecycleStage) params.set('lifecycle_stage', lifecycleStage);
    const qs = params.toString() ? `?${params.toString()}` : '';

    const result = await call('GET', `/heuristics/rules${qs}`, null, args);
    output(formatHeuristicRuleList(result, args.mode), args);
  });

  register('get-rule', async (args) => {
    const ruleId = requirePositional(args, 'rule-id', 0);
    const result = await call('GET', `/heuristics/rules/${encodeURIComponent(ruleId)}`, null, args) as Record<string, unknown>;
    output(formatHeuristicRuleDetail(result, args.mode), args);
  });

  register('promote-rule', async (args) => {
    const ruleId = requirePositional(args, 'rule-id', 0);
    const newStage = optStr(args, 'new-stage') ?? 'candidate';

    const result = await call('POST', `/heuristics/rules/${encodeURIComponent(ruleId)}/promote`, {
      new_stage: newStage,
    }, args) as Record<string, unknown>;
    output(formatHeuristicRulePromoted(result, args.mode), args);
  });

  // ── Instance CRUD ────────────────────────────────────────────────
  register('create-instance', async (args) => {
    const contextSummary = requirePositional(args, 'context-summary', 0);
    const ruleId = optStr(args, 'rule-id');
    const userReaction = optStr(args, 'user-reaction') ?? 'neutral';
    const signalType = optStr(args, 'signal-type') ?? 'explicit_negation';
    const tags = splitComma(optStr(args, 'tags'));
    const agentProposal = optStr(args, 'agent-proposal');
    const outcome = optStr(args, 'outcome');
    const instanceSessionId = optStr(args, 'session-id');

    const result = await call('POST', '/heuristics/instances', {
      context_summary: contextSummary,
      // rule_id not in server CreateHeuristicInstanceRequest — stored via tags instead
      ...(ruleId || tags.length > 0 ? { tags: ruleId ? [...tags, `rule:${ruleId}`] : tags } : {}),
      user_reaction: userReaction,
      signal_type: signalType,
      ...(agentProposal ? { agent_proposal: agentProposal } : {}),
      ...(outcome ? { outcome } : {}),
      ...(instanceSessionId ? { session_id: instanceSessionId } : {}),
      user_id: args.config.userId,
    }, args) as Record<string, unknown>;

    output(formatHeuristicInstanceCreated(result, args.mode), args);
  });

  register('list-instances', async (args) => {
    const ruleId = optStr(args, 'rule-id');
    const params = new URLSearchParams();
    if (ruleId) params.set('rule_id', ruleId);
    const qs = params.toString() ? `?${params.toString()}` : '';

    const result = await call('GET', `/heuristics/instances${qs}`, null, args);
    output(formatHeuristicInstanceList(result, args.mode), args);
  });

  register('get-instance', async (args) => {
    const instanceId = requirePositional(args, 'instance-id', 0);
    const result = await call('GET', `/heuristics/instances/${encodeURIComponent(instanceId)}`, null, args) as Record<string, unknown>;
    output(formatHeuristicInstanceDetail(result, args.mode), args);
  });

  // ── Retrieve ──────────────────────────────────────────────────────
  register('retrieve', async (args) => {
    const query = requirePositional(args, 'intent', 0);
    const tags = splitComma(optStr(args, 'tags'));
    const topK = optNum(args, 'top-k') ?? 10;

    const result = await call('POST', '/heuristics/retrieve', {
      query, ...(tags.length > 0 ? { tags } : {}), top_k: topK, user_id: args.config.userId,
    }, args) as Record<string, unknown>;

    output(formatHeuristicRetrieve(result, args.mode), args);
  });
}