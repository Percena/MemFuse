import {
  CliArgs, RegisterFn, call, output, requirePositional,
  optNum, optStr, optBool, sessionId,
} from '../types.js';
import {
  formatSession, formatSessionList, formatSessionContext, formatSessionDelete,
  formatSessionArchive, formatExportMemories,
} from '../output.js';

export function registerSessionCommands(register: RegisterFn): void {
  register('session-create', async (args) => {
    const sessionIdOpt = args.config.sessionId !== 'default' ? args.config.sessionId : undefined;
    const result = await call('POST', '/sessions', {
      session_id: sessionIdOpt || undefined,
    }, args) as Record<string, unknown>;
    output(formatSession(result, args.mode), args);
  });

  register('session-list', async (args) => {
    const result = await call('GET', '/sessions', null, args);
    output(formatSessionList(result, args.mode), args);
  });

  register('session-get', async (args) => {
    const sessionIdArg = requirePositional(args, 'session-id', 0);
    const result = await call('GET', `/sessions/${encodeURIComponent(sessionIdArg)}`, null, args) as Record<string, unknown>;
    output(formatSession(result, args.mode), args);
  });

  register('session-context', async (args) => {
    const sessionIdArg = requirePositional(args, 'session-id', 0);
    const tokenBudget = optNum(args, 'token-budget');
    const qs = tokenBudget !== undefined ? `?token_budget=${tokenBudget}` : '';

    const result = await call('GET',
      `/sessions/${encodeURIComponent(sessionIdArg)}/context${qs}`,
      null, args) as Record<string, unknown>;

    output(formatSessionContext(result, args.mode), args);
  });

  register('session-delete', async (args) => {
    const sessionIdArg = requirePositional(args, 'session-id', 0);
    const result = await call('DELETE', `/sessions/${encodeURIComponent(sessionIdArg)}`, null, args) as Record<string, unknown>;
    output(formatSessionDelete(sessionIdArg, result, args.mode), args);
  });

  register('session-archive', async (args) => {
    const sessionIdArg = requirePositional(args, 'session-id', 0);
    const archiveId = requirePositional(args, 'archive-id', 1);
    const result = await call('GET',
      `/sessions/${encodeURIComponent(sessionIdArg)}/archives/${encodeURIComponent(archiveId)}`,
      null, args) as Record<string, unknown>;
    output(formatSessionArchive(result, args.mode), args);
  });

  // ── Session internal ops ─────────────────────────────────────────
  register('add-message', async (args) => {
    const sid = requirePositional(args, 'session-id', 0);
    const role = optStr(args, 'role') ?? 'user';
    const content = requirePositional(args, 'content', 1);

    const result = await call('POST',
      `/sessions/${encodeURIComponent(sid)}/messages`,
      { role, content }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(`Message added (${role}) to session ${sid}`, args);
  });

  register('used-context', async (args) => {
    const sid = requirePositional(args, 'session-id', 0);
    const uri = requirePositional(args, 'uri', 1);

    const result = await call('POST',
      `/sessions/${encodeURIComponent(sid)}/used_context`,
      { uri }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(`Context usage recorded: ${uri}`, args);
  });

  register('used-skill', async (args) => {
    const sid = requirePositional(args, 'session-id', 0);
    const skillUri = requirePositional(args, 'skill-uri', 1);
    const success = optBool(args, 'success') ?? true;

    const result = await call('POST',
      `/sessions/${encodeURIComponent(sid)}/used_skill`,
      { skill_uri: skillUri, success }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(`Skill usage recorded: ${skillUri} (success: ${success})`, args);
  });

  register('used-tool', async (args) => {
    const sid = requirePositional(args, 'session-id', 0);
    const toolUri = requirePositional(args, 'tool-uri', 1);
    const success = optBool(args, 'success') ?? true;

    const result = await call('POST',
      `/sessions/${encodeURIComponent(sid)}/used_tool`,
      { tool_uri: toolUri, success }, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    output(`Tool usage recorded: ${toolUri} (success: ${success})`, args);
  });

  register('session-timeline', async (args) => {
    const sid = requirePositional(args, 'session-id', 0);

    const result = await call('GET',
      `/sessions/${encodeURIComponent(sid)}/timeline`,
      null, args) as Record<string, unknown>;

    if (args.mode === 'json') { output(JSON.stringify(result), args); return; }
    const timeline = Array.isArray(result.timeline) ? result.timeline : [];
    const count = Number(result.count ?? timeline.length);
    if (timeline.length === 0) { output('No timeline events.', args); return; }
    const lines = timeline.map((ep: Record<string, unknown>) =>
      `  ${String(ep.archive_id ?? '')} (${Number(ep.message_count ?? 0)} msgs) ${String(ep.abstract ?? '')}`
    );
    output(`Timeline (${count} events):\n${lines.join('\n')}`, args);
  });
}