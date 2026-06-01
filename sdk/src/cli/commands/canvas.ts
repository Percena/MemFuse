import {
  CliArgs, RegisterFn, call, output, requirePositional, optStr,
} from '../types.js';

export function registerCanvasCommands(register: RegisterFn): void {
  const manifestGet = async (args: CliArgs) => {
    const repoId = optStr(args, 'repo') ?? args.positional[0];

    const qp = repoId ? `repo_id=${encodeURIComponent(repoId)}` : '';
    const result = await call('GET', `/manifest/get?${qp}`, null, args) as Record<string, unknown>;
    const data = unwrapData(result);
    const repoIdentity = record(data.repo_identity);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    if (!repoIdentity.repo_id) {
      output(`No manifest found${repoId ? ` for repo_id: ${repoId}` : ' (no repo_id specified, tried account_id fallback)'}`, args);
      return;
    }

    const lines = [
      `Manifest for ${repoIdentity.repo_id}`,
      `  resource_uri:      ${repoIdentity.resource_uri}`,
      `  default_branch:    ${repoIdentity.default_branch}`,
      `  primary_languages: ${formatJsonValue(repoIdentity.primary_languages)}`,
      `  manifest_yaml_path: ${data.manifest_yaml_path}`,
      `  last_verified_at:  ${repoIdentity.last_verified_at}`,
      `  created_at:        ${repoIdentity.created_at}`,
    ];
    if (data.hint != null) lines.push(`  Hint:              ${data.hint}`);
    output(lines.join('\n'), args);
  };

  register('manifest-get', manifestGet);
  register('repo-manifest', manifestGet);

  register('manifest-update', async (args) => {
    const repoId = requirePositional(args, 'repo-id', 0);
    const resourceUri = optStr(args, 'resource-uri') ?? '';
    const defaultBranch = optStr(args, 'default-branch') ?? 'main';
    const primaryLanguages = optStr(args, 'primary-languages');
    const manifestYamlPath = optStr(args, 'manifest-yaml-path') ?? 'manifest.yaml';

    const result = await call('POST', '/manifest/update', {
      repo_id: repoId,
      resource_uri: resourceUri,
      default_branch: defaultBranch,
      primary_languages: primaryLanguages ? parseJsonValue(args, primaryLanguages, 'primary-languages') : undefined,
      manifest_yaml_path: manifestYamlPath,
      updater: 'human',
    }, args) as Record<string, unknown>;

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    output(`Manifest updated for ${result.repo_id}`, args);
  });

  register('canvas-query', async (args) => {
    const repoId = optStr(args, 'repo') ?? requirePositional(args, 'repo-id', 0);
    const component = optStr(args, 'component');
    const canvasType = optStr(args, 'type');
    const nodeType = optStr(args, 'node-type');
    const status = optStr(args, 'status');

    const params = new URLSearchParams({ repo_id: repoId });
    if (component) params.set('component', component);
    if (canvasType) params.set('type', canvasType);
    if (nodeType) params.set('node_type', nodeType);
    if (status) params.set('status', status);

    const result = await call('GET', `/canvas/query?${params.toString()}`, null, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    const nodes = Array.isArray(data.nodes) ? data.nodes : [];
    const edges = Array.isArray(data.edges) ? data.edges : [];
    const overlays = Array.isArray(data.overlays) ? data.overlays : [];
    const conflicts = Array.isArray(data.conflicts) ? data.conflicts : [];

    const lines = [
      `Canvas for ${repoId}`,
      `  Nodes: ${nodes.length}  Edges: ${edges.length}  Overlays: ${overlays.length}  Conflicts: ${conflicts.length}`,
    ];
    for (const n of nodes as Record<string, unknown>[]) {
      lines.push(`  [${n.node_type}] ${n.name} (${n.id})`);
    }
    for (const o of overlays as Record<string, unknown>[]) {
      lines.push(`  Overlay [${o.overlay_type}] ${o.id} status=${o.status}`);
    }
    for (const c of conflicts as Record<string, unknown>[]) {
      lines.push(`  Conflict: ${c.overlay_a} ↔ ${c.overlay_b}`);
    }
    if (data.hint != null) lines.push(`  Hint: ${data.hint}`);
    output(lines.join('\n'), args);
  });

  register('canvas-refresh', async (args) => {
    const repoId = optStr(args, 'repo') ?? requirePositional(args, 'repo-id', 0);

    const result = await call('POST', '/canvas/refresh', { repo_id: repoId }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    output(`Canvas refreshed for ${repoId}. Nodes: ${data.nodes_count}, Edges: ${data.edges_count}`, args);
  });

  register('canvas-snapshot', async (args) => {
    const repoId = optStr(args, 'repo') ?? requirePositional(args, 'repo-id', 0);
    const snapshotType = optStr(args, 'snapshot-type') ?? 'full';
    const mergeCommit = optStr(args, 'merge-commit') ?? 'HEAD';

    const result = await call('POST', '/canvas/snapshot', {
      repo_id: repoId,
      merge_commit: mergeCommit,
      snapshot_type: snapshotType,
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    output(`Snapshot created: ${data.snapshot_id} for ${repoId}`, args);
  });

  register('overlay-propose', async (args) => {
    const repoId = optStr(args, 'repo') ?? requirePositional(args, 'repo-id', 0);
    const overlayType = optStr(args, 'type') ?? requirePositional(args, 'overlay-type', 1);
    const contentJsonRaw = optStr(args, 'content-json') ?? requirePositional(args, 'content-json', 2);
    const branch = optStr(args, 'branch');
    const affectedNodes = optStr(args, 'affected-nodes');
    const affectedEdges = optStr(args, 'affected-edges');
    const tracker = optStr(args, 'tracker') ?? 'github_projects';
    const trackerContentId = requiredOption(args, optStr(args, 'content-id') ?? optStr(args, 'tracker-content-id'), 'content-id');
    const trackerIdentifier = requiredOption(args, optStr(args, 'identifier') ?? optStr(args, 'tracker-identifier'), 'identifier');
    const author = optStr(args, 'author') ?? args.config.userId;

    const result = await call('POST', '/overlay/propose', {
      repo_id: repoId,
      overlay_type: overlayType,
      content_json: parseJsonValue(args, contentJsonRaw, 'content-json'),
      branch,
      affected_nodes: parseStringArray(args, affectedNodes, 'affected-nodes'),
      affected_edges: parseStringArray(args, affectedEdges, 'affected-edges'),
      tracker,
      tracker_content_id: trackerContentId,
      tracker_identifier: trackerIdentifier,
      author,
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    const proposeMsg = `Overlay proposed: ${data.overlay_id} [${overlayType}] status=${data.status}`;
    if (data.version_hash != null) {
      output(`${proposeMsg}\nVersion: ${data.version_hash}`, args);
    } else {
      output(proposeMsg, args);
    }
  });

  register('overlay-accept', async (args) => {
    const overlayId = requirePositional(args, 'overlay-id', 0);

    const result = await call('POST', '/overlay/accept', {
      overlay_id: overlayId,
      acceptor: 'human',
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    const acceptMsg = `Overlay ${overlayId} accepted. New status: ${data.status ?? data.new_status}`;
    if (data.version_hash != null) {
      output(`${acceptMsg}\nVersion: ${data.version_hash}`, args);
    } else {
      output(acceptMsg, args);
    }
  });

  register('overlay-implement', async (args) => {
    const overlayId = requirePositional(args, 'overlay-id', 0);
    const agentSessionId = optStr(args, 'agent-session-id');

    const result = await call('POST', '/overlay/mark_implemented', {
      overlay_id: overlayId,
      agent_session_id: agentSessionId,
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    output(`Overlay ${overlayId} marked implemented. New status: ${data.status ?? data.new_status}`, args);
  });

  register('overlay-abandon', async (args) => {
    const overlayId = requirePositional(args, 'overlay-id', 0);
    const reason = optStr(args, 'reason') ?? 'Abandoned';
    const abandoner = optStr(args, 'abandoner') ?? optStr(args, 'actor') ?? 'human';

    const result = await call('POST', '/overlay/abandon', {
      overlay_id: overlayId,
      reason,
      abandoner,
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    output(`Overlay ${overlayId} abandoned. New status: ${data.status ?? data.new_status}`, args);
  });

  register('overlay-conflict', async (args) => {
    const repoId = requiredOption(args, optStr(args, 'repo'), 'repo');
    const overlayId1 = optStr(args, 'overlay-a') ?? requirePositional(args, 'overlay-id-1', 0);
    const overlayId2 = optStr(args, 'overlay-b') ?? requirePositional(args, 'overlay-id-2', 1);
    const conflictDescription = optStr(args, 'description') ?? optStr(args, 'conflict-description');

    const result = await call('POST', '/overlay/report_conflict', {
      repo_id: repoId,
      overlay_id_1: overlayId1,
      overlay_id_2: overlayId2,
      conflict_description: conflictDescription,
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    const hasConflict = data.has_conflict === true || data.has_conflict === 'true' || data.has_overlap === true;
    output(`Conflict check: ${overlayId1} ↔ ${overlayId2} — has_conflict=${hasConflict}`, args);
  });

  register('overlay-consolidate', async (args) => {
    const repoId = optStr(args, 'repo') ?? requirePositional(args, 'repo-id', 0);
    const mergeCommit = optStr(args, 'merge-commit') ?? 'HEAD';

    const result = await call('POST', '/overlay/consolidate', {
      repo_id: repoId,
      merge_commit: mergeCommit,
    }, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    output(`Consolidation complete for ${repoId}. Merged overlays: ${data.merged_count}, Snapshot: ${data.snapshot_id}`, args);
  });

  register('overlays', async (args) => {
    const repoId = requirePositional(args, 'repo-id', 0);
    const status = optStr(args, 'status');

    const params = new URLSearchParams({ repo_id: repoId });
    if (status) params.set('status', status);

    const result = await call('GET', `/overlays?${params.toString()}`, null, args) as Record<string, unknown>;
    const data = unwrapData(result);

    if (args.mode === 'json') {
      output(JSON.stringify(result, null, 2), args);
      return;
    }

    const overlays = Array.isArray(data.overlays) ? data.overlays : Array.isArray(data.items) ? data.items : [];
    const lines = [`Overlays for ${repoId} (${overlays.length})`];
    for (const o of overlays as Record<string, unknown>[]) {
      lines.push(`  [${o.overlay_type}] ${o.id} status=${o.status} author=${o.author}`);
    }
    output(lines.join('\n'), args);
  });
}

function unwrapData(result: Record<string, unknown>): Record<string, unknown> {
  const data = result.data;
  if (data && typeof data === 'object' && !Array.isArray(data)) {
    return data as Record<string, unknown>;
  }
  return result;
}

function record(value: unknown): Record<string, unknown> {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? value as Record<string, unknown>
    : {};
}

function formatJsonValue(value: unknown): string {
  return typeof value === 'string' ? value : JSON.stringify(value ?? null);
}

function requiredOption(args: CliArgs, value: string | undefined, name: string): string {
  if (!value) {
    console.error(`Error: ${args.command} requires --${name}.`);
    process.exit(1);
  }
  return value;
}

function parseJsonValue(args: CliArgs, raw: string, name: string): unknown {
  try {
    return JSON.parse(raw);
  } catch (_) {
    console.error(`Error: --${name} must be valid JSON.`);
    process.exit(1);
  }
}

function parseStringArray(args: CliArgs, raw: string | undefined, name: string): string[] {
  if (!raw) return [];
  try {
    const parsed = JSON.parse(raw);
    if (Array.isArray(parsed) && parsed.every(item => typeof item === 'string')) {
      return parsed;
    }
  } catch (_) {
    // Fall back to comma-separated values below.
  }
  const values = raw.split(',').map(item => item.trim()).filter(Boolean);
  if (values.length > 0) return values;
  console.error(`Error: --${name} must be a JSON string array or comma-separated list.`);
  process.exit(1);
}
