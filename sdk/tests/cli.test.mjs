/**
 * MemFuse CLI Tests — Command routing, output formatting, arg parsing
 *
 * Tests run against compiled dist output.
 * Run: node --test tests/cli.test.mjs
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import http from 'node:http';
import { execFile } from 'node:child_process';
import { promisify } from 'node:util';
import { mkdtemp, access, mkdir, readFile, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

const execFileAsync = promisify(execFile);

// ─── 1. Output format module ────────────────────────────────────────

describe('CLI Output Formatting', () => {
  it('formatSearchResults default mode — compact, no emoji', async () => {
    const { formatSearchResults } = await import('../dist/cli/output.js');
    const result = {
      total: 2,
      results: [
        { episode_id: 'ep1', summary: 'Auth discussion', score: 0.85, salience_score: 0.7, created_at: '2026-04-28T10:00:00Z' },
        { episode_id: 'ep2', summary: 'Migration work', score: 0.72, salience_score: 0.6, created_at: '2026-04-27T08:00:00Z' },
      ],
    };
    const output = formatSearchResults(result, 'default');
    assert.ok(!output.includes('##'), 'default mode should not have markdown headers');
    assert.ok(!output.includes('📁'), 'default mode should not have emoji');
    assert.ok(output.includes('ep1'), 'should include episode id');
    assert.ok(output.includes('0.85'), 'should include score');
    assert.ok(output.includes('r:0.85'), 'compact format uses r: prefix for relevance');
  });

  it('formatSearchResults verbose mode — full markdown', async () => {
    const { formatSearchResults } = await import('../dist/cli/output.js');
    const result = {
      total: 1,
      results: [
        { episode_id: 'ep1', summary: 'Auth system', score: 0.92, salience_score: 0.8, created_at: '2026-04-28T10:00:00Z' },
      ],
    };
    const output = formatSearchResults(result, 'verbose');
    assert.ok(output.includes('## Search Results'), 'verbose mode should have headers');
    assert.ok(output.includes('**'), 'verbose mode should have bold');
    assert.ok(output.includes('relevance'), 'verbose mode should spell out relevance');
  });

  it('formatSearchResults json mode — raw JSON', async () => {
    const { formatSearchResults } = await import('../dist/cli/output.js');
    const result = { total: 0, results: [] };
    const output = formatSearchResults(result, 'json');
    const parsed = JSON.parse(output);
    assert.equal(parsed.total, 0);
  });

  it('formatFacts default mode — compact', async () => {
    const { formatFacts } = await import('../dist/cli/output.js');
    const facts = [
      { subject: 'User', predicate: 'prefers', display_value: 'snake_case', confidence: 0.92 },
      { subject: 'Project', predicate: 'language', display_value: 'Rust', confidence: 0.88 },
    ];
    const output = formatFacts(facts, 'default');
    assert.ok(output.includes('User → prefers: snake_case'), 'should include subject→predicate:value');
    assert.ok(output.includes('(0.92)'), 'should include confidence');
    assert.ok(!output.includes('##'), 'default mode no markdown headers');
  });

  it('formatFacts verbose mode — markdown with confidence markers', async () => {
    const { formatFacts } = await import('../dist/cli/output.js');
    const facts = [
      { subject: 'User', predicate: 'prefers', display_value: 'snake_case', confidence: 0.95 },
    ];
    const output = formatFacts(facts, 'verbose');
    assert.ok(output.includes('## Active Facts'), 'verbose mode has header');
    assert.ok(output.includes('✓'), 'high confidence uses ✓ marker');
  });

  it('formatLs default mode — terse entries with URI', async () => {
    const { formatLs } = await import('../dist/cli/output.js');
    const entries = [
      { name: 'api.md', is_dir: false },
      { name: 'guides', is_dir: true },
    ];
    const output = formatLs('mfs://resources/docs', entries, 'default');
    assert.ok(output.includes('- mfs://resources/docs/api.md'), 'file entries use - prefix with full URI');
    assert.ok(output.includes('d mfs://resources/docs/guides'), 'directory entries use d prefix with full URI');
    assert.ok(!output.includes('📁'), 'default mode no emoji');
  });

  it('formatLs verbose mode — emoji + URIs', async () => {
    const { formatLs } = await import('../dist/cli/output.js');
    const entries = [
      { name: 'api.md', is_dir: false },
    ];
    const output = formatLs('mfs://resources/docs', entries, 'verbose');
    assert.ok(output.includes('📄'), 'verbose mode uses file emoji');
    assert.ok(output.includes('mfs://resources/docs/api.md'), 'verbose mode includes full URI');
  });

  it('formatTimeline default mode — compact', async () => {
    const { formatTimeline } = await import('../dist/cli/output.js');
    const episodes = [
      { episode_id: 'ep-1', summary: 'Auth work', score: 0.8, created_at: '2026-04-27T10:00:00Z' },
      { episode_id: 'anchor', summary: 'Current session', score: 0.9, created_at: '2026-04-28T10:00:00Z' },
    ];
    const facts = [{ subject: 'Project', predicate: 'uses', display_value: 'SQLite' }];
    const output = formatTimeline('anchor', episodes, facts, 'default');
    assert.ok(output.includes('>>> anchor'), 'anchor episode marked with >>>');
    assert.ok(!output.includes('▶'), 'default mode no verbose markers');
  });

  it('formatTimeline verbose mode — full markers', async () => {
    const { formatTimeline } = await import('../dist/cli/output.js');
    const episodes = [
      { episode_id: 'anchor', summary: 'Current', score: 0.9, created_at: '2026-04-28T10:00:00Z' },
    ];
    const output = formatTimeline('anchor', episodes, [], 'verbose');
    assert.ok(output.includes('▶ **[NOW]**'), 'verbose mode has NOW marker');
  });

  it('formatObservations default mode — compact', async () => {
    const { formatObservations } = await import('../dist/cli/output.js');
    const details = [
      { episode_id: 'ep-1', summary: 'Decision made', facts: [{ subject: 'A', predicate: 'B', display_value: 'C' }], created_at: '2026-04-28T10:00:00Z' },
    ];
    const output = formatObservations(details, 'default');
    assert.ok(output.includes('ep-1'), 'includes episode id');
    assert.ok(output.includes('A → B: C'), 'includes fact triple');
  });

  it('formatText — simple text output', async () => {
    const { formatText } = await import('../dist/cli/output.js');
    const output = formatText('Abstract', 'mfs://resources/docs/api.md', 'API docs summary', 'default');
    assert.equal(output, 'API docs summary');
    const verbose = formatText('Abstract', 'mfs://resources/docs/api.md', 'API docs', 'verbose');
    assert.ok(verbose.includes('## Abstract'));
  });

  it('formatGlob — file listing', async () => {
    const { formatGlob } = await import('../dist/cli/output.js');
    const matches = ['api.md', 'guide.md', 'test.rs'];
    const output = formatGlob('mfs://resources/docs', '*.md', matches, 'default');
    assert.ok(output.includes('api.md'));
    assert.ok(!output.includes('##'), 'default mode no headers');
    const verbose = formatGlob('mfs://resources/docs', '*.md', matches, 'verbose');
    assert.ok(verbose.includes('## Glob Matches'));
  });

  it('OutputMode type exported', async () => {
    const mod = await import('../dist/cli/output.js');
    assert.equal(typeof mod.formatSearchResults, 'function');
    assert.equal(typeof mod.formatFacts, 'function');
    assert.equal(typeof mod.formatLs, 'function');
    assert.equal(typeof mod.formatTimeline, 'function');
    assert.equal(typeof mod.formatObservations, 'function');
    assert.equal(typeof mod.formatText, 'function');
    assert.equal(typeof mod.formatGlob, 'function');
    assert.equal(typeof mod.formatResolveContext, 'function');
    assert.equal(typeof mod.formatContextSearchResults, 'function');
    assert.equal(typeof mod.formatSession, 'function');
    assert.equal(typeof mod.formatSessionList, 'function');
    assert.equal(typeof mod.formatObservationStored, 'function');
    assert.equal(typeof mod.formatSessionCommitted, 'function');
    assert.equal(typeof mod.formatResourceAdded, 'function');
    assert.equal(typeof mod.formatResourceList, 'function');
    assert.equal(typeof mod.formatTaskStatus, 'function');
    assert.equal(typeof mod.formatTaskList, 'function');
    assert.equal(typeof mod.formatCiteMemories, 'function');
    assert.equal(typeof mod.formatImportMemories, 'function');
    assert.equal(typeof mod.formatConfirmRule, 'function');
    assert.equal(typeof mod.formatSkillsList, 'function');
    assert.equal(typeof mod.formatHealth, 'function');
    assert.equal(typeof mod.formatSystemStatus, 'function');
    assert.equal(typeof mod.formatObserverStatus, 'function');
    assert.equal(typeof mod.stageStr, 'function');
  });

  it('stageStr returns correct markers', async () => {
    const { stageStr } = await import('../dist/cli/output.js');
    assert.equal(stageStr('confirmed', 'default'), '[confirmed]');
    assert.equal(stageStr('candidate', 'default'), '[candidate]');
    assert.equal(stageStr('draft', 'default'), '[draft]');
    assert.equal(stageStr('confirmed', 'verbose'), '★');
    assert.equal(stageStr('candidate', 'verbose'), '◆');
    assert.equal(stageStr('draft', 'verbose'), '○');
  });

  it('formatResolveContext default strips verbose markers', async () => {
    const { formatResolveContext } = await import('../dist/cli/output.js');
    const result = {
      rendered_markdown: '## Current Facts\n✓ **User** → prefers: snake_case\n---\n*Use `get_observations` for more detail.*',
    };
    const output = formatResolveContext(result, 'default');
    assert.ok(!output.includes('##'), 'default strips markdown headers');
    assert.ok(!output.includes('✓'), 'default strips emoji');
    assert.ok(!output.includes('Use `get_observations`'), 'default strips footer tips');
  });

  it('formatResolveContext verbose keeps full markdown', async () => {
    const { formatResolveContext } = await import('../dist/cli/output.js');
    const result = {
      rendered_markdown: '## Current Facts\n✓ **User** → prefers: snake_case\n---\n*Tip: Use search for more.*',
    };
    const output = formatResolveContext(result, 'verbose');
    assert.equal(output, result.rendered_markdown, 'verbose mode passes rendered_markdown through');
  });

  it('formatResolveContext json mode returns raw JSON', async () => {
    const { formatResolveContext } = await import('../dist/cli/output.js');
    const result = { rendered_markdown: 'test', sections: {} };
    const output = formatResolveContext(result, 'json');
    const parsed = JSON.parse(output);
    assert.equal(parsed.rendered_markdown, 'test');
  });

  it('formatResolveContext fallback when no rendered_markdown', async () => {
    const { formatResolveContext } = await import('../dist/cli/output.js');
    const result = {
      sections: {
        current_facts: [{ subject: 'User', predicate: 'prefers', display_value: 'snake_case', confidence: 0.92 }],
        relevant_history: [{ episode_id: 'ep1', summary: 'Auth work', score: 0.85 }],
        recent_updates: ['Session started'],
        behavioral_heuristics: [{ lifecycle_stage: 'confirmed', rule_text: 'Prefer fallback paths', tags: 'domain:arch' }],
      },
    };
    const output = formatResolveContext(result, 'default');
    assert.ok(output.includes('User → prefers: snake_case'), 'fallback includes facts');
    assert.ok(output.includes('[confirmed]'), 'fallback includes heuristics stage markers');
  });

  it('formatObservationStored default mode — compact', async () => {
    const { formatObservationStored } = await import('../dist/cli/output.js');
    const result = { turn_id: 'turn-1', auto_committed: true };
    const output = formatObservationStored(result, 'default');
    assert.ok(output.includes('Stored'), 'default mode says Stored');
    assert.ok(output.includes('auto-committed'), 'default mode shows auto-committed');
  });

  it('formatObservationStored verbose mode — full markdown', async () => {
    const { formatObservationStored } = await import('../dist/cli/output.js');
    const result = { turn_id: 'turn-1', auto_committed: true };
    const output = formatObservationStored(result, 'verbose');
    assert.ok(output.includes('##'), 'verbose mode has header');
    assert.ok(output.includes('✓'), 'verbose mode has checkmark');
  });

  it('formatSessionCommitted default mode — compact', async () => {
    const { formatSessionCommitted } = await import('../dist/cli/output.js');
    const result = { archive_uri: 'archive://ep-1', task_id: 'task-1' };
    const output = formatSessionCommitted(result, 'default');
    assert.ok(output.includes('Committed'), 'default says Committed');
    assert.ok(output.includes('archive://ep-1'), 'default includes archive URI');
  });

  it('formatSession default mode — compact', async () => {
    const { formatSession } = await import('../dist/cli/output.js');
    const result = { session_id: 'sess-1', status: 'active', turn_count: 5 };
    const output = formatSession(result, 'default');
    assert.ok(output.includes('sess-1'), 'includes session id');
    assert.ok(!output.includes('##'), 'no markdown headers');
  });

  it('formatCiteMemories default mode — compact', async () => {
    const { formatCiteMemories } = await import('../dist/cli/output.js');
    const result = { cited_episodes: 2, cited_facts: 3 };
    const output = formatCiteMemories(result, 'default');
    assert.ok(output.includes('2'), 'includes episode count');
    assert.ok(output.includes('3'), 'includes fact count');
  });

  it('formatImportMemories default mode — compact', async () => {
    const { formatImportMemories } = await import('../dist/cli/output.js');
    const result = { updated_facts: 10, retracted_facts: 2, total_imported: 15 };
    const output = formatImportMemories(result, 'default');
    assert.ok(output.includes('15'), 'includes total count');
    assert.ok(!output.includes('##'), 'no headers in default');
  });

  it('formatHealth default mode — compact', async () => {
    const { formatHealth } = await import('../dist/cli/output.js');
    const result = { status: 'alive', version: '0.1.0' };
    const output = formatHealth(result, 'http://127.0.0.1:8720', 'default');
    assert.ok(output.includes('alive'), 'includes status');
    assert.ok(output.includes('0.1.0'), 'includes version');
    assert.ok(!output.includes('##'), 'no headers in default');
  });

  it('formatHealth verbose mode — full markdown', async () => {
    const { formatHealth } = await import('../dist/cli/output.js');
    const result = { status: 'alive', version: '0.1.0' };
    const output = formatHealth(result, 'http://127.0.0.1:8720', 'verbose');
    assert.ok(output.includes('## MemFuse Health'), 'verbose has header');
    assert.ok(output.includes('alive'), 'verbose includes status');
  });

  it('formatSystemStatus default mode — compact key-value', async () => {
    const { formatSystemStatus } = await import('../dist/cli/output.js');
    const result = { workspace_root: '/tmp/ws', source_kind: 'localfs' };
    const output = formatSystemStatus(result, 'default');
    assert.ok(output.includes('workspace_root'), 'includes key');
    assert.ok(!output.includes('##'), 'no headers in default');
  });

  it('formatResourceAdded default mode — compact', async () => {
    const { formatResourceAdded } = await import('../dist/cli/output.js');
    const result = { resource_id: 'res-1', task_key: 'task-1' };
    const output = formatResourceAdded(result, 'default');
    assert.ok(output.includes('res-1'), 'includes resource id');
    assert.ok(!output.includes('##'), 'no headers');
  });

  it('formatResourceOp — refresh/rebuild compact', async () => {
    const { formatResourceOp } = await import('../dist/cli/output.js');
    const result = { task_key: 'task-refresh-1' };
    const output = formatResourceOp('Refresh', result, 'default');
    assert.ok(output.includes('Refresh'), 'includes operation label');
    assert.ok(output.includes('task-refresh-1'), 'includes task key');
    assert.ok(!output.includes('##'), 'no headers in default');
  });

  it('formatResourceOp — verbose mode', async () => {
    const { formatResourceOp } = await import('../dist/cli/output.js');
    const result = { task_key: 'task-rebuild-1' };
    const output = formatResourceOp('Rebuild', result, 'verbose');
    assert.ok(output.includes('## Rebuild'), 'verbose has header');
    assert.ok(output.includes('task-rebuild-1'), 'verbose includes task key');
  });

  it('formatSimulateReaction default mode', async () => {
    const { formatSimulateReaction } = await import('../dist/cli/output.js');
    const result = {
      relevant_rules: [{ lifecycle_stage: 'confirmed', rule_text: 'Use TDD', tags: 'testing' }],
      prediction: 'User will approve',
    };
    const output = formatSimulateReaction(result, 'default');
    assert.ok(output.includes('[confirmed]'), 'default mode uses text markers');
    assert.ok(output.includes('Use TDD'), 'includes rule text');
    assert.ok(output.includes('Prediction'), 'includes prediction');
    assert.ok(!output.includes('##'), 'no headers');
  });

  it('formatSimulateReaction verbose mode', async () => {
    const { formatSimulateReaction } = await import('../dist/cli/output.js');
    const result = {
      relevant_rules: [{ lifecycle_stage: 'confirmed', rule_text: 'Use TDD', tags: 'testing' }],
      prediction: 'User will approve',
    };
    const output = formatSimulateReaction(result, 'verbose');
    assert.ok(output.includes('★'), 'verbose mode uses emoji markers');
    assert.ok(output.includes('## Simulate Reaction'), 'verbose has header');
  });

  it('formatHeuristicsL0 default mode', async () => {
    const { formatHeuristicsL0 } = await import('../dist/cli/output.js');
    const result = [{ rule_text: 'Prefer fallback', tags: 'domain:arch' }];
    const output = formatHeuristicsL0(result, 'default');
    assert.ok(output.includes('[confirmed]'), 'default shows confirmed marker');
    assert.ok(output.includes('Prefer fallback'), 'includes rule text');
  });

  it('formatHeuristicsL0 verbose mode', async () => {
    const { formatHeuristicsL0 } = await import('../dist/cli/output.js');
    const result = [{ rule_text: 'Prefer fallback', tags: 'domain:arch' }];
    const output = formatHeuristicsL0(result, 'verbose');
    assert.ok(output.includes('★'), 'verbose uses emoji');
    assert.ok(output.includes('## Confirmed Rules'), 'verbose has header');
  });

  it('formatSessionContext default mode', async () => {
    const { formatSessionContext } = await import('../dist/cli/output.js');
    const result = {
      messages: [{ role: 'user', content: 'What about auth?' }, { role: 'assistant', content: 'Auth is implemented' }],
      latest_archive_overview: 'Previous session about testing',
    };
    const output = formatSessionContext(result, 'default');
    assert.ok(output.includes('[user]'), 'default includes role');
    assert.ok(!output.includes('##'), 'no headers');
  });

  it('formatSessionContext verbose mode', async () => {
    const { formatSessionContext } = await import('../dist/cli/output.js');
    const result = {
      messages: [{ role: 'user', content: 'What about auth?' }],
    };
    const output = formatSessionContext(result, 'verbose');
    assert.ok(output.includes('## Session Context'), 'verbose has header');
    assert.ok(output.includes('**[user]**'), 'verbose bolds role');
  });

  it('formatSessionDelete default mode', async () => {
    const { formatSessionDelete } = await import('../dist/cli/output.js');
    const output = formatSessionDelete('sess-1', { deleted: true }, 'default');
    assert.ok(output.includes('Deleted'), 'includes deleted');
    assert.ok(!output.includes('##'), 'no headers');
  });

  it('formatSessionDelete verbose mode', async () => {
    const { formatSessionDelete } = await import('../dist/cli/output.js');
    const output = formatSessionDelete('sess-1', { deleted: true }, 'verbose');
    assert.ok(output.includes('##'), 'verbose has header');
  });

  it('formatTextRaw json mode passes through full object', async () => {
    const { formatTextRaw } = await import('../dist/cli/output.js');
    const result = { uri: 'mfs://docs', raw: 'Hello world', label: 'Read' };
    const output = formatTextRaw(result, 'json');
    const parsed = JSON.parse(output);
    assert.equal(parsed.raw, 'Hello world');
    assert.equal(parsed.uri, 'mfs://docs');
  });

  it('formatTaskStatus verbose mode', async () => {
    const { formatTaskStatus } = await import('../dist/cli/output.js');
    const result = { status: 'completed', processing_mode: 'full' };
    const output = formatTaskStatus(result, 'verbose');
    assert.ok(output.includes('## Task Status'), 'verbose has header');
  });

  it('formatCiteMemories verbose mode', async () => {
    const { formatCiteMemories } = await import('../dist/cli/output.js');
    const result = { cited_episodes: 2, cited_facts: 3 };
    const output = formatCiteMemories(result, 'verbose');
    assert.ok(output.includes('## Citation Recorded'), 'verbose has header');
    assert.ok(output.includes('✓'), 'verbose has checkmarks');
  });

  it('formatImportMemories verbose mode', async () => {
    const { formatImportMemories } = await import('../dist/cli/output.js');
    const result = { updated_facts: 10, retracted_facts: 2, total_imported: 15 };
    const output = formatImportMemories(result, 'verbose');
    assert.ok(output.includes('## Import Complete'), 'verbose has header');
  });

  it('formatSkillsList default mode', async () => {
    const { formatSkillsList } = await import('../dist/cli/output.js');
    const result = [{ name: 'memfuse', uri: '/skills/memfuse' }];
    const output = formatSkillsList(result, 'default');
    assert.ok(output.includes('memfuse'), 'includes skill name');
    assert.ok(!output.includes('##'), 'no headers');
  });

  // ─── New formatter tests ──────────────────────────────────────────

  it('formatFactCreated default mode', async () => {
    const { formatFactCreated } = await import('../dist/cli/output.js');
    const result = { fact_id: 'fact-1' };
    const output = formatFactCreated(result, 'default');
    assert.ok(output.includes('Created'), 'default mode says Created');
    assert.ok(output.includes('fact-1'), 'includes fact id');
  });

  it('formatFactSuperseded default mode', async () => {
    const { formatFactSuperseded } = await import('../dist/cli/output.js');
    const result = { old_fact_id: 'fact-old', new_fact_id: 'fact-new' };
    const output = formatFactSuperseded(result, 'default');
    assert.ok(output.includes('Superseded'), 'default mode says Superseded');
  });

  it('formatFactRetracted default mode', async () => {
    const { formatFactRetracted } = await import('../dist/cli/output.js');
    const result = { fact_id: 'fact-1', retracted: true };
    const output = formatFactRetracted(result, 'default');
    assert.ok(output.includes('retracted'), 'default mode confirms retracted');
  });

  it('formatHeuristicRuleCreated default mode', async () => {
    const { formatHeuristicRuleCreated } = await import('../dist/cli/output.js');
    const result = { rule_id: 'rule-1', lifecycle_stage: 'draft' };
    const output = formatHeuristicRuleCreated(result, 'default');
    assert.ok(output.includes('Created'), 'says Created');
    assert.ok(output.includes('[draft]'), 'shows stage');
  });

  it('formatHeuristicRuleList default mode', async () => {
    const { formatHeuristicRuleList } = await import('../dist/cli/output.js');
    const result = [{ rule_id: 'r1', rule_text: 'Use TDD', lifecycle_stage: 'confirmed', tags: 'testing' }];
    const output = formatHeuristicRuleList(result, 'default');
    assert.ok(output.includes('[confirmed]'), 'shows stage marker');
  });

  it('formatRelationLink default mode', async () => {
    const { formatRelationLink } = await import('../dist/cli/output.js');
    const result = { from_uri: 'mfs://a', to_uri: 'mfs://b', relation_type: 'related' };
    const output = formatRelationLink(result, 'default');
    assert.ok(output.includes('Linked'), 'says Linked');
  });

  it('formatWorkspaceWrite default mode', async () => {
    const { formatWorkspaceWrite } = await import('../dist/cli/output.js');
    const result = { ok: true };
    const output = formatWorkspaceWrite('mkdir', 'mfs://new-dir', result, 'default');
    assert.ok(output.includes('ok'), 'shows ok');
  });

  it('formatWatchList default mode', async () => {
    const { formatWatchList } = await import('../dist/cli/output.js');
    const result = [{ watch_id: 'w1', resource_id: 'res-1', status: 'active' }];
    const output = formatWatchList(result, 'default');
    assert.ok(output.includes('w1'), 'watch id shown');
  });

  it('formatCodeSymbolsSearch default mode', async () => {
    const { formatCodeSymbolsSearch } = await import('../dist/cli/output.js');
    const result = { results: [{ name: 'AuthService', uri: 'mfs://docs/auth.rs', score: 0.92 }] };
    const output = formatCodeSymbolsSearch(result, 'default');
    assert.ok(output.includes('AuthService'), 'symbol name shown');
  });

  it('formatSessionArchive default mode', async () => {
    const { formatSessionArchive } = await import('../dist/cli/output.js');
    const result = { overview: 'Previous session about testing' };
    const output = formatSessionArchive(result, 'default');
    assert.ok(output.includes('Previous session about testing'), 'overview shown');
  });

  it('formatExportMemories default mode shows compact summary', async () => {
    const { formatExportMemories } = await import('../dist/cli/output.js');
    const result = { markdown: '# Facts\n...', fact_count: 10, rule_count: 5 };
    const output = formatExportMemories(result, 'default');
    assert.ok(output.includes('10'), 'includes fact count');
    assert.ok(output.includes('5'), 'includes rule count');
    assert.ok(output.startsWith('<!--'), 'starts with HTML comment for pipeline compatibility');
    assert.ok(output.includes('# Facts'), 'includes markdown content');
  });
});

// ─── 2. CLI bin entry point ──────────────────────────────────────────

describe('CLI Bin Entry Point', () => {
  it('--help prints help text without error', async () => {
    const { stdout, stderr } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', '--help'], {
      cwd: new URL('..', import.meta.url).pathname,
    });
    assert.ok(stdout.includes('resolve-context'), 'help should list resolve-context');
    assert.ok(stdout.includes('store-observation'), 'help should list store-observation');
    assert.ok(stdout.includes('LOOK → DIG → SAVE'), 'help should mention workflow');
    assert.ok(stdout.includes('--json'), 'help should list --json');
    assert.ok(stdout.includes('--verbose'), 'help should list --verbose');
    assert.equal(stderr, '', 'no stderr errors');
  });

  it('--version prints version', async () => {
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', '--version'], {
      cwd: new URL('..', import.meta.url).pathname,
    });
    assert.ok(stdout.includes('memfuse v'), 'version should start with memfuse v');
  });

  it('unknown command exits with error', async () => {
    try {
      await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'nonexistent-command'], {
        cwd: new URL('..', import.meta.url).pathname,
      });
      assert.fail('Should have thrown');
    } catch (err) {
      assert.ok(err.stderr.includes('Unknown command'), 'should say unknown command');
      assert.ok(err.stderr.includes('--help'), 'should suggest --help');
    }
  });

  it('no arguments prints help', async () => {
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs'], {
      cwd: new URL('..', import.meta.url).pathname,
    });
    assert.ok(stdout.includes('resolve-context'), 'empty args prints help');
  });

  it('memfuse-setup accepts documented install alias', async () => {
    const projectDir = await mkdtemp(join(tmpdir(), 'memfuse-setup-install-'));
    const env = {
      ...process.env,
      MEMFUSE_SKIP_MCP: '1',
      PATH: '',
    };

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse-setup.cjs',
      'install',
      '--platform=codex',
      `--project-dir=${projectDir}`,
      '--server-url=http://127.0.0.1:9',
    ], {
      cwd: new URL('..', import.meta.url).pathname,
      env,
    });

    assert.match(stdout, /Codex installation complete/);
    await access(join(projectDir, '.codex', 'skills', 'memfuse', 'SKILL.md'));
    await access(join(projectDir, '.codex', 'skills', 'memfuse', 'references', 'commands.md'));
    await access(join(projectDir, '.codex-plugin', 'plugin.json'));
  });

  it('health command connects to server and reports status', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ status: 'alive', version: '0.1.0' }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'health', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('alive'), 'health should report alive status');
    assert.ok(stdout.includes('0.1.0'), 'health should report version');
  });

  it('service status reports local supervisor guidance without contacting server', async () => {
    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'service', 'status',
    ], { cwd: new URL('..', import.meta.url).pathname });

    assert.match(stdout, /MemFuse service/);
    assert.match(stdout, /systemd|launchd|manual/i);
  });

  it('service install writes a deterministic systemd user unit and config', async () => {
    const home = await mkdtemp(join(tmpdir(), 'memfuse-service-home-'));
    const xdgConfig = join(home, '.config');
    const xdgData = join(home, '.local', 'share');
    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: xdgConfig,
      XDG_DATA_HOME: xdgData,
      MEMFUSE_SERVICE_PLATFORM: 'linux',
      MEMFUSE_SERVER_BIN: '/opt/memfuse/bin/memfuse-server',
    };

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs',
      'service',
      'install',
      '--scope=user',
    ], { cwd: new URL('..', import.meta.url).pathname, env });

    const unit = await readFile(join(xdgConfig, 'systemd', 'user', 'memfuse.service'), 'utf-8');
    const config = await readFile(join(xdgConfig, 'memfuse', 'config.toml'), 'utf-8');
    assert.match(stdout, /installed/);
    assert.match(unit, /ExecStart=\/opt\/memfuse\/bin\/memfuse-server --config/);
    assert.match(unit, /Restart=on-failure/);
    assert.match(config, /\[client\]/);
    assert.match(config, /server_url = "http:\/\/127\.0\.0\.1:18720"/);
  });

  it('service doctor reports binary, config, data dir, and health status', async () => {
    const home = await mkdtemp(join(tmpdir(), 'memfuse-service-doctor-'));
    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: join(home, '.config'),
      XDG_DATA_HOME: join(home, '.local', 'share'),
      MEMFUSE_SERVICE_PLATFORM: 'linux',
      MEMFUSE_SERVER_BIN: process.execPath,
      MEMFUSE_SERVER_URL: 'http://127.0.0.1:9',
    };

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs',
      'service',
      'doctor',
    ], { cwd: new URL('..', import.meta.url).pathname, env });

    assert.match(stdout, /binary: ok/);
    assert.match(stdout, /config: missing/);
    assert.match(stdout, /data_dir: missing/);
    assert.match(stdout, /ready: offline/);
  });

  it('service doctor reads server_url from MemFuse config when env is absent', async () => {
    const home = await mkdtemp(join(tmpdir(), 'memfuse-service-doctor-config-'));
    const xdgConfig = join(home, '.config');
    const configDir = join(xdgConfig, 'memfuse');
    await mkdir(configDir, { recursive: true });
    await writeFile(join(configDir, 'config.toml'), `
[client]
server_url = "http://127.0.0.1:9"
`);
    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: xdgConfig,
      XDG_DATA_HOME: join(home, '.local', 'share'),
      MEMFUSE_SERVICE_PLATFORM: 'linux',
      MEMFUSE_SERVER_BIN: process.execPath,
    };
    delete env.MEMFUSE_SERVER_URL;
    delete env.MEMFUSE_CONFIG;
    delete env.MEMFUSE_WORKSPACE_ROOT;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs',
      'service',
      'doctor',
    ], { cwd: new URL('..', import.meta.url).pathname, env });

    assert.match(stdout, /ready: offline \(http:\/\/127\.0\.0\.1:9\/ready\)/);
  });

  it('service doctor uses MEMFUSE_CONFIG as the effective config path', async () => {
    const home = await mkdtemp(join(tmpdir(), 'memfuse-service-doctor-env-config-'));
    const customConfig = join(home, 'custom-config.toml');
    const customDataDir = join(home, 'custom-data');
    await mkdir(customDataDir, { recursive: true });
    await writeFile(customConfig, `
[storage]
data_dir = "${customDataDir}"

[client]
server_url = "http://127.0.0.1:9"
`);
    const env = {
      ...process.env,
      HOME: home,
      XDG_CONFIG_HOME: join(home, '.config'),
      XDG_DATA_HOME: join(home, '.local', 'share'),
      MEMFUSE_CONFIG: customConfig,
      MEMFUSE_SERVICE_PLATFORM: 'linux',
      MEMFUSE_SERVER_BIN: process.execPath,
    };
    delete env.MEMFUSE_SERVER_URL;
    delete env.MEMFUSE_WORKSPACE_ROOT;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs',
      'service',
      'doctor',
    ], { cwd: new URL('..', import.meta.url).pathname, env });

    assert.match(stdout, new RegExp(`config: ok \\(${escapeRegExp(customConfig)}\\)`));
    assert.match(stdout, new RegExp(`data_dir: ok \\(${escapeRegExp(customDataDir)}\\)`));
    assert.match(stdout, /ready: offline \(http:\/\/127\.0\.0\.1:9\/ready\)/);
  });

  it('search command calls server and formats results', async () => {
    let seenBody;
    const server = http.createServer((req, res) => {
      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          seenBody = JSON.parse(body || '{}');
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            total: 1,
            results: [{ episode_id: 'ep-auth', summary: 'Auth system discussion', score: 0.92, salience_score: 0.8, created_at: '2026-04-28T10:00:00Z' }],
          }));
        });
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'search', 'auth', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.equal(seenBody.query, 'auth');
    assert.equal(seenBody.session_id, undefined);
    assert.ok(stdout.includes('ep-auth'), 'search should include episode id');
    assert.ok(stdout.includes('Auth system discussion'), 'search should include summary');
  });

  it('search command includes session scope only when explicitly requested', async () => {
    let seenBody;
    const server = http.createServer((req, res) => {
      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          seenBody = JSON.parse(body || '{}');
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ total: 0, results: [] }));
        });
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'search', 'auth',
      `--server=http://127.0.0.1:${port}`,
      '--session=session-explicit',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.equal(seenBody.query, 'auth');
    assert.equal(seenBody.session_id, 'session-explicit');
  });

  it('resolve-context calls server and formats output', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/context/resolve' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          rendered_markdown: '## Current Facts\n✓ **User** → prefers: snake_case',
          sections: {
            current_facts: [{ subject: 'User', predicate: 'prefers', display_value: 'snake_case', confidence: 0.92 }],
            recent_updates: [],
            relevant_history: [],
          },
        }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'resolve-context', 'what am I working on?',
      `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('User → prefers: snake_case'), 'default mode should strip verbose markers');
  });

  it('resolve-context --verbose keeps full markdown', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/context/resolve' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          rendered_markdown: '## Current Facts\n✓ **User** → prefers: snake_case\n---\n*Tip: dig deeper*',
        }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'resolve-context', 'test query',
      `--server=http://127.0.0.1:${port}`, '--verbose',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('## Current Facts'), 'verbose keeps headers');
    assert.ok(stdout.includes('✓'), 'verbose keeps emoji');
    assert.ok(stdout.includes('Tip'), 'verbose keeps tips');
  });

  it('search --json returns raw JSON', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/v1/memory:search' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ total: 0, results: [] }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'search', 'test', `--server=http://127.0.0.1:${port}`, '--json',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    const parsed = JSON.parse(stdout);
    assert.equal(parsed.total, 0);
  });

  it('list-facts calls server and formats output', async () => {
    const server = http.createServer((req, res) => {
      if (req.url.startsWith('/facts')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [
            { subject: 'User', predicate: 'prefers', display_value: 'snake_case for Rust', confidence: 0.92 },
          ],
          next_cursor: null,
          total_count: 1,
          limit: 100,
        }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'list-facts', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('User → prefers'), 'default mode includes fact triple');
    assert.ok(!stdout.includes('##'), 'default mode no headers');
  });

  it('store-observation sends correct payload', async () => {
    let receivedBody = null;
    const server = http.createServer((req, res) => {
      if (req.url.startsWith('/sessions') && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ turn_id: 'turn-1', auto_committed: true }));
        });
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'store-observation',
      'Decided to use async consolidation', '--type', 'Decision',
      `--server=http://127.0.0.1:${port}`, '--session', 'test-session',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.equal(receivedBody.content, 'Decided to use async consolidation');
    assert.equal(receivedBody.tool_name, 'Decision');
    assert.equal(receivedBody.platform, 'cli');
    assert.ok(stdout.toLowerCase().includes('stored'), 'should confirm storage');
  });

  it('commit-session sends correct payload', async () => {
    let receivedUrl = null;
    const server = http.createServer((req, res) => {
      receivedUrl = req.url;
      if (req.url === '/sessions/default/commit' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ archive_uri: 'archive://ep-1', task_id: 'task-1' }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'commit-session', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(receivedUrl.includes('/commit'), 'should call commit endpoint');
    assert.ok(stdout.toLowerCase().includes('committed'), 'should confirm commit');
  });

  it('session-list calls server and formats output', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/sessions' && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [
            { session_id: 'sess-1', status: 'active' },
            { session_id: 'sess-2', status: 'committed' },
          ],
          next_cursor: null,
          total_count: 2,
          limit: 20,
        }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'session-list', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('sess-1'), 'should list sessions');
    assert.ok(stdout.includes('active'), 'should show status');
  });

  it('session-delete calls server and formats output', async () => {
    const server = http.createServer((req, res) => {
      if (req.url.startsWith('/sessions/sess-1') && req.method === 'DELETE') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ deleted: true }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'session-delete', 'sess-1', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.toLowerCase().includes('deleted'), 'should confirm delete');
  });

  it('health --verbose formats full markdown', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/health') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ status: 'alive', version: '0.1.0' }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'health', `--server=http://127.0.0.1:${port}`, '--verbose',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('## MemFuse Health'), 'verbose mode should have header');
  });

  it('resources-list calls server and formats output', async () => {
    const server = http.createServer((req, res) => {
      if (req.url === '/resources' && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [
            { resource_id: 'res-1', logical_name: 'docs', status: 'ready' },
          ],
          next_cursor: null,
          total_count: 1,
          limit: 100,
        }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'resources-list', `--server=http://127.0.0.1:${port}`,
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    assert.ok(stdout.includes('res-1'), 'should list resources');
    assert.ok(!stdout.includes('##'), 'default mode no headers');
  });

  it('read --json returns full server response', async () => {
    const server = http.createServer((req, res) => {
      if (req.url.startsWith('/read')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ raw: 'Hello world content', uri: 'mfs://docs/test.md' }));
        return;
      }
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'read', 'mfs://docs/test.md', `--server=http://127.0.0.1:${port}`, '--json',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
    const parsed = JSON.parse(stdout);
    assert.equal(parsed.raw, 'Hello world content');
    assert.equal(parsed.uri, 'mfs://docs/test.md');
  });
});

function escapeRegExp(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

// ─── 3. Arg parsing edge cases ──────────────────────────────────────

describe('CLI Arg Parsing', () => {
  it('--session flag overrides default session', async () => {
    const server = http.createServer((req, res) => {
      // Verify the session ID is in the URL
      assert.ok(req.url.includes('my-custom-session'), 'session override should be in URL');
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ turn_id: 'turn-1', auto_committed: true }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'store-observation', 'test content',
      `--server=http://127.0.0.1:${port}`, '--session', 'my-custom-session',
    ], { cwd: new URL('..', import.meta.url).pathname });

    server.close();
  });

  it('--server flag overrides MEMFUSE_SERVER_URL', async () => {
    const server = http.createServer((req, res) => {
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ status: 'alive', version: '0.1.0' }));
    });

    await new Promise(resolve => server.listen(0, '127.0.0.1', resolve));
    const address = server.address();
    const port = typeof address === 'object' && address ? address.port : 0;

    // Set env var to a different server, then override via --server
    const { stdout } = await execFileAsync(process.execPath, [
      'bin/memfuse.cjs', 'health', `--server=http://127.0.0.1:${port}`,
    ], {
      cwd: new URL('..', import.meta.url).pathname,
      env: { ...process.env, MEMFUSE_SERVER_URL: 'http://127.0.0.1:9999' },
    });

    server.close();
    assert.ok(stdout.includes('alive'), '--server override should work');
  });
});

// ─── 4. Error handling ──────────────────────────────────────────────

describe('CLI Error Handling', () => {
  it('missing required positional arg exits with error', async () => {
    try {
      await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'resolve-context'], {
        cwd: new URL('..', import.meta.url).pathname,
      });
      assert.fail('Should have thrown');
    } catch (err) {
      assert.ok(err.stderr.includes('requires query'), 'should explain missing arg');
    }
  });

  it('server unreachable exits with error', async () => {
    try {
      await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'health', '--server=http://127.0.0.1:1',
      ], {
        cwd: new URL('..', import.meta.url).pathname,
        timeout: 5000,
      });
      assert.fail('Should have thrown');
    } catch (err) {
      assert.ok(err.stderr.includes('not reachable') || err.stderr.includes('Error'), 'should report server error');
    }
  });
});

// ─── 5. Extended Integration Tests ────────────────────────────────────

function makeServer(portRef) {
  let handler = null;
  const server = http.createServer((req, res) => {
    if (handler) {
      handler(req, res);
    } else {
      res.writeHead(404);
      res.end(JSON.stringify({ error: 'not found' }));
    }
  });
  return {
    setup: (h) => { handler = h; },
    start: () => new Promise(resolve => server.listen(0, '127.0.0.1', resolve)),
    port: () => { const a = server.address(); return typeof a === 'object' && a ? a.port : 0; },
    close: () => server.close(),
  };
}

const cwd = new URL('..', import.meta.url).pathname;

describe('CLI Extended Integration', () => {
  it('ls calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/ls')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify([{ name: 'docs', is_dir: true }, { name: 'api.md', is_dir: false }]));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'ls', 'mfs://resources/docs', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('d'), 'directory entries have d prefix');
    assert.ok(stdout.includes('api.md'), 'file entries listed');
  });

  it('abstract calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/abstract')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ raw: 'Summary of the document' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'abstract', 'mfs://resources/docs/api.md', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('Summary of the document'), 'abstract shown');
  });

  it('task-status calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/tasks/')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ status: 'completed', processing_mode: 'full' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'task-status', 'task-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.toLowerCase().includes('done'), 'completed task shows done');
  });

  it('tasks-list calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/tasks')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ task_id: 'task-1', status: 'completed' }, { task_id: 'task-2', status: 'running' }],
          next_cursor: null,
          limit: 20,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'tasks-list', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('task-1'), 'first task listed');
    assert.ok(stdout.includes('task-2'), 'second task listed');
  });

  it('snapshots calls server and formats paginated output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/snapshots')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ snapshot_id: 'snap-1', created_at: '2026-05-07T00:00:00Z' }],
          next_cursor: null,
          total_count: 1,
          limit: 50,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'snapshots', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('snap-1'), 'snapshot listed');
  });

  it('audit calls server and formats paginated output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/audit')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ timestamp: '2026-05-07T00:00:00Z', action: 'resource.read', resource_id: 'docs' }],
          next_cursor: null,
          total_count: 1,
          limit: 50,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'audit', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('resource.read'), 'audit action listed');
  });

  it('heuristics-l0 calls server with POST and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/heuristics/l0-confirmed' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify([{ rule_text: 'Prefer TDD', tags: 'testing' }]));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'heuristics-l0', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('[confirmed]'), 'shows confirmed marker');
    assert.ok(stdout.includes('Prefer TDD'), 'shows rule text');
  });

  it('simulate-reaction calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/heuristics/simulate-reaction' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          const parsed = JSON.parse(body || '{}');
          assert.equal(parsed.scenario, 'adding new feature');
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ relevant_rules: [{ lifecycle_stage: 'confirmed', rule_text: 'Use TDD', tags: 'testing' }], prediction: 'User approves' }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'simulate-reaction', 'adding new feature', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('[confirmed]'), 'shows confirmed marker');
    assert.ok(stdout.includes('Prediction'), 'shows prediction');
  });

  it('confirm-rule calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/heuristics/rules/') && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ user_confirmed: true }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'confirm-rule', 'rule-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.toLowerCase().includes('confirmed'), 'confirms rule');
  });

  it('session-create calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/sessions' && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ session_id: 'new-session', status: 'active' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'session-create', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('new-session'), 'session id shown');
  });

  it('session-get calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/sessions/sess-1') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ session_id: 'sess-1', status: 'active', turn_count: 3 }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'session-get', 'sess-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('sess-1'), 'session id shown');
    assert.ok(!stdout.includes('##'), 'default mode no headers');
  });

  it('system-status calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/system/status') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ workspace_root: '/tmp/ws', source_kind: 'localfs' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'system-status', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('workspace_root'), 'key shown');
    assert.ok(!stdout.includes('##'), 'default mode no headers');
  });

  it('observer-status calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/system/observer') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ running: true, total_ticks: 100 }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'observer-status', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('running'), 'key shown');
  });

  it('cite-memories calls server with POST', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/memories/cite' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          const parsed = JSON.parse(body || '{}');
          assert.deepEqual(parsed.episode_ids, ['ep1', 'ep2']);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ cited_episodes: 2, cited_facts: 1 }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'cite-memories', `--server=http://127.0.0.1:${srv.port()}`, '--episode-ids=ep1,ep2'], { cwd });
    srv.close();
    assert.ok(stdout.toLowerCase().includes('cited'), 'shows citation');
  });

  it('add-repo calls server with correct payload', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/resources' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ resource_id: 'res-git', task_key: 'task-git' }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'add-repo', 'https://github.com/example/repo', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.equal(receivedBody.source_kind, 'git_url');
    assert.equal(receivedBody.source_path, 'https://github.com/example/repo');
    assert.ok(stdout.includes('res-git'), 'resource id shown');
  });

  it('skills-list calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/skills' && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ name: 'memfuse', uri: '/skills/memfuse' }],
          skills: [{ name: 'memfuse', uri: '/skills/memfuse' }],
          next_cursor: null,
          total_count: 1,
          count: 1,
          limit: 100,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'skills-list', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('memfuse'), 'skill name shown');
  });

  it('get-observations calls server for comma-separated IDs', async () => {
    const srv = makeServer(null);
    let pathsSeen = [];
    srv.setup((req, res) => {
      pathsSeen.push(req.url);
      if (req.url.startsWith('/episodes/')) {
        const id = req.url.split('/episodes/')[1].split('?')[0];
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ episode_id: id, summary: `Summary of ${id}`, facts: [], created_at: '2026-04-28T10:00:00Z' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'get-observations', 'ep1,ep2', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.equal(pathsSeen.length, 2, 'fetches two episodes');
    assert.ok(stdout.includes('ep1'), 'ep1 shown');
    assert.ok(stdout.includes('ep2'), 'ep2 shown');
  });

  it('list-facts --json returns raw JSON', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/facts')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ facts: [{ subject: 'Project', predicate: 'lang', display_value: 'Rust', confidence: 0.95 }] }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'list-facts', `--server=http://127.0.0.1:${srv.port()}`, '--json'], { cwd });
    srv.close();
    const parsed = JSON.parse(stdout);
    assert.ok(parsed.facts, 'json mode passes full response');
    assert.equal(parsed.facts[0].subject, 'Project');
  });

  it('--api-key sends Authorization header', async () => {
    let receivedHeaders = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      receivedHeaders = req.headers;
      res.writeHead(200, { 'Content-Type': 'application/json' });
      res.end(JSON.stringify({ status: 'alive', version: '0.1.0' }));
    });
    await srv.start();
    await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'health', `--server=http://127.0.0.1:${srv.port()}`, '--api-key=my-secret-key'], { cwd });
    srv.close();
    assert.equal(receivedHeaders.authorization, 'Bearer my-secret-key', 'api key sent as Bearer');
  });

  it('MEMFUSE_THREAD_ID maps to session', async () => {
    const srv = makeServer(null);
    let receivedUrl = null;
    let receivedBody = null;
    srv.setup((req, res) => {
      receivedUrl = req.url;
      let body = '';
      req.on('data', chunk => { body += chunk; });
      req.on('end', () => {
        receivedBody = JSON.parse(body || '{}');
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ turn_id: 'turn-1', auto_committed: false }));
      });
    });
    await srv.start();
    await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'store-observation', 'test', `--server=http://127.0.0.1:${srv.port()}`], { cwd, env: { ...process.env, MEMFUSE_THREAD_ID: 'thread-abc' } });
    srv.close();
    assert.ok(receivedUrl.includes('thread-abc'), 'MEMFUSE_THREAD_ID used as session');
    assert.equal(receivedBody.tool_input, '', 'text-only observations include required empty tool_input');
    assert.equal(receivedBody.tool_output, '', 'text-only observations include required empty tool_output');
  });

  // ─── New Command Tests ──────────────────────────────────────────

  it('session-archive calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/sessions/sess-1/archives/arch-1')) {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ overview: 'Session about auth refactoring', messages: [{ role: 'user', content: 'How does auth work?' }] }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'session-archive', 'sess-1', 'arch-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('auth refactoring'), 'overview shown');
  });

  it('create-fact calls server with POST and formats output', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/facts' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ fact_id: 'fact-new', subject: 'Project', predicate: 'lang', display_value: 'Rust' }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'create-fact', 'Project', 'lang', 'Rust', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.equal(receivedBody.subject, 'Project');
    assert.equal(receivedBody.predicate, 'lang');
    assert.equal(receivedBody.display_value, 'Rust');
    assert.ok(stdout.includes('fact-new'), 'fact id shown');
  });

  it('supersede-fact calls server with POST', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/facts/fact-old/supersede') && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ old_fact_id: 'fact-old', new_fact_id: 'fact-new' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'supersede-fact', 'fact-old', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('Superseded'), 'supersede confirmed');
  });

  it('retract-fact calls server with POST', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/facts/fact-bad/retract') && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ fact_id: 'fact-bad', retracted: true }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'retract-fact', 'fact-bad', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('retracted'), 'retract confirmed');
  });

  it('create-rule calls server with POST and formats output', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/heuristics/rules' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ rule_id: 'rule-1', rule_text: 'Use TDD', lifecycle_stage: 'draft' }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'create-rule', 'Use TDD', '--tags', 'domain:testing,phase:dev', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.equal(receivedBody.rule_text, 'Use TDD');
    assert.deepEqual(receivedBody.tags, ['domain:testing', 'phase:dev']);
    assert.ok(stdout.includes('rule-1'), 'rule id shown');
  });

  it('list-rules calls server with GET and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/heuristics/rules') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ rule_id: 'rule-1', rule_text: 'Use TDD', lifecycle_stage: 'confirmed', tags: 'testing' }],
          rules: [{ rule_id: 'rule-1', rule_text: 'Use TDD', lifecycle_stage: 'confirmed', tags: 'testing' }],
          next_cursor: null,
          total_count: 1,
          total: 1,
          limit: 100,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'list-rules', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('[confirmed]'), 'shows lifecycle stage');
    assert.ok(stdout.includes('Use TDD'), 'shows rule text');
  });

  it('list-instances calls server with GET and formats paginated output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/heuristics/instances') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ instance_id: 'inst-1', context_summary: 'Review feedback', signal_type: 'preference_declaration' }],
          instances: [{ instance_id: 'inst-1', context_summary: 'Review feedback', signal_type: 'preference_declaration' }],
          next_cursor: null,
          total_count: 1,
          total: 1,
          limit: 100,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'list-instances', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('inst-1'), 'instance id shown');
    assert.ok(stdout.includes('Review feedback'), 'context shown');
  });

  it('get-rule calls server with GET and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/heuristics/rules/rule-1') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ rule_id: 'rule-1', rule_text: 'Use TDD', lifecycle_stage: 'confirmed', tags: 'testing', evidence_count: 5 }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'get-rule', 'rule-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('rule-1'), 'rule id shown');
    assert.ok(stdout.includes('Use TDD'), 'rule text shown');
  });

  it('promote-rule calls server with POST', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/heuristics/rules/rule-1/promote') && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ rule_id: 'rule-1', lifecycle_stage: 'candidate', new_stage: 'confirmed' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'promote-rule', 'rule-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('promoted'), 'promote confirmed');
  });

  it('retrieve calls server with POST and formats output', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/heuristics/retrieve' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ heuristics: [{ rule_id: 'rule-1', rule_text: 'Prefer fallback', lifecycle_stage: 'confirmed', tags: ['domain:arch'], aggregate_weight: 0.85 }], total: 1 }));
        });
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'retrieve', 'architecture decisions', '--tags', 'domain:arch', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
      assert.equal(receivedBody.query, 'architecture decisions');
      assert.ok(stdout.includes('Prefer fallback'), 'rule text shown');
    } finally {
      srv.close();
    }
  });

  it('link calls server with POST and formats output', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/relations' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ from_uri: 'mfs://docs/a', to_uri: 'mfs://docs/b', relation_type: 'related' }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'link', 'mfs://docs/a', 'mfs://docs/b', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.equal(receivedBody.from_uri, 'mfs://docs/a');
    assert.equal(receivedBody.to_uri, 'mfs://docs/b');
    assert.ok(stdout.includes('Linked'), 'link confirmed');
  });

  it('unlink calls server with DELETE and query params', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/relations?') && req.method === 'DELETE') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ from_uri: 'mfs://docs/a', to_uri: 'mfs://docs/b', relation_type: 'related' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'unlink', 'mfs://docs/a', 'mfs://docs/b', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('Unlinked'), 'unlink confirmed');
  });

  it('relations calls server with GET and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/relations') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ direction: 'outbound', peer_uri: 'mfs://docs/b', relation_type: 'related' }],
          relations: [{ direction: 'outbound', peer_uri: 'mfs://docs/b', relation_type: 'related' }],
          next_cursor: null,
          total_count: 1,
          count: 1,
          limit: 20,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'relations', 'mfs://docs/a', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('outbound'), 'direction shown');
    assert.ok(stdout.includes('mfs://docs/b'), 'peer uri shown');
  });

  it('mkdir calls server with POST and JSON body', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/mkdir' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ ok: true }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'mkdir', 'mfs://docs/new-dir', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('ok'), 'mkdir confirmed');
    assert.strictEqual(receivedBody.uri, 'mfs://docs/new-dir', 'uri sent in body');
  });

  it('write calls server with POST and content', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/write' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({ ok: true }));
        });
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'write', 'mfs://docs/file.md', '--content', 'Hello world', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.equal(receivedBody.uri, 'mfs://docs/file.md');
    assert.equal(receivedBody.content, 'Hello world');
    assert.ok(stdout.includes('ok'), 'write confirmed');
  });

  it('rm calls server with DELETE', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/rm') && req.method === 'DELETE') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ ok: true }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'rm', 'mfs://docs/old-file.md', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('ok'), 'rm confirmed');
  });

  it('watches-list calls server and formats output', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/watches' && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ watch_id: 'w1', resource_id: 'res-1', status: 'active' }],
          watches: [{ watch_id: 'w1', resource_id: 'res-1', status: 'active' }],
          next_cursor: null,
          total_count: 1,
          count: 1,
          limit: 100,
        }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'watches-list', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('w1'), 'watch id shown');
  });

  it('resource-watch calls server with POST', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/resources/res-1/watch') && req.method === 'POST') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({ status: 'registered' }));
      }
    });
    await srv.start();
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', 'resource-watch', 'res-1', `--server=http://127.0.0.1:${srv.port()}`], { cwd });
    srv.close();
    assert.ok(stdout.includes('registered'), 'watch registered');
  });

  it('code-symbols-search calls server with GET', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/code_symbols/search') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ symbol_name: 'AuthService', canonical_uri: 'mfs://docs/auth.rs', score: 0.92 }],
          symbols: [{ symbol_name: 'AuthService', canonical_uri: 'mfs://docs/auth.rs', score: 0.92 }],
          results: [{ symbol_name: 'AuthService', canonical_uri: 'mfs://docs/auth.rs', score: 0.92 }],
          next_cursor: null,
          total_count: 1,
          count: 1,
          limit: 100,
        }));
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'code-symbols-search', 'auth',
        '--projection-view-id', 'view-1',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd });
      assert.ok(stdout.includes('AuthService'), 'symbol name shown');
    } finally {
      srv.close();
    }
  });

  it('code-symbols-list calls server with GET', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url.startsWith('/code_symbols') && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          items: [{ id: 'sym-auth', symbol_name: 'AuthService', canonical_uri: 'mfs://docs/auth.rs' }],
          symbols: [{ id: 'sym-auth', symbol_name: 'AuthService', canonical_uri: 'mfs://docs/auth.rs' }],
          next_cursor: null,
          total_count: 1,
          count: 1,
          limit: 100,
        }));
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'code-symbols-list',
        '--projection-view-id', 'view-1',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd });
      assert.ok(stdout.includes('AuthService'), 'symbol name shown');
    } finally {
      srv.close();
    }
  });

  it('repo-manifest unwraps the repo-intelligence envelope', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/manifest/get?repo_id=symphony-gh' && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          status: 'ok',
          data: {
            repo_identity: {
              repo_id: 'symphony-gh',
              resource_uri: 'mfs://resources/localfs/symphony-gh/MANIFEST.yaml',
              default_branch: 'main',
              primary_languages: ['elixir'],
              created_at: '2026-05-11T00:00:00Z',
              last_verified_at: '2026-05-11T00:00:00Z',
            },
            manifest_yaml_path: '/tmp/MANIFEST.yaml',
            canvas_indexes: [{ type: 'structural', version_hash: 'abc' }],
          },
        }));
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'repo-manifest', '--repo', 'symphony-gh',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd });
      assert.ok(stdout.includes('symphony-gh'), 'repo id shown');
      assert.ok(stdout.includes('mfs://resources/localfs/symphony-gh/MANIFEST.yaml'), 'resource uri shown');
    } finally {
      srv.close();
    }
  });

  it('canvas-query uses PRD query flags and unwraps the envelope', async () => {
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/canvas/query?repo_id=symphony-gh&component=Runner&type=structural' && req.method === 'GET') {
        res.writeHead(200, { 'Content-Type': 'application/json' });
        res.end(JSON.stringify({
          status: 'ok',
          data: {
            nodes: [{ id: 'n1', node_type: 'module', name: 'Runner' }],
            edges: [{ id: 'e1' }],
            overlays: [{ id: 'o1', overlay_type: 'planned_change', status: 'proposed' }],
            conflicts: [],
          },
        }));
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'canvas-query', '--repo', 'symphony-gh',
        '--component', 'Runner', '--type', 'structural',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd });
      assert.ok(stdout.includes('Runner'), 'node shown');
      assert.ok(stdout.includes('o1'), 'overlay shown');
    } finally {
      srv.close();
    }
  });

  it('overlay-propose sends PRD tracker fields, author, and JSON content', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/overlay/propose' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            status: 'ok',
            data: { overlay_id: 'overlay-1', status: 'proposed' },
          }));
        });
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'overlay-propose',
        '--repo', 'symphony-gh',
        '--tracker', 'github_projects',
        '--content-id', 'I_kwDO_1',
        '--identifier', 'owner/repo#1',
        '--type', 'planned_change',
        '--content-json', '{"summary":"change runner"}',
        '--affected-nodes', '["n1"]',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd, env: { ...process.env, MEMFUSE_USER_ID: 'alice' } });
      assert.equal(receivedBody.repo_id, 'symphony-gh');
      assert.equal(receivedBody.tracker_content_id, 'I_kwDO_1');
      assert.equal(receivedBody.tracker_identifier, 'owner/repo#1');
      assert.equal(receivedBody.author, 'alice');
      assert.deepEqual(receivedBody.content_json, { summary: 'change runner' });
      assert.deepEqual(receivedBody.affected_nodes, ['n1']);
      assert.ok(stdout.includes('overlay-1'), 'overlay id shown');
    } finally {
      srv.close();
    }
  });

  it('overlay-abandon sends human abandoner by default', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/overlay/abandon' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            status: 'ok',
            data: { overlay_id: 'overlay-1', status: 'abandoned', triggered_by: 'human' },
          }));
        });
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'overlay-abandon',
        'overlay-1',
        '--reason', 'PR closed',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd });
      assert.equal(receivedBody.overlay_id, 'overlay-1');
      assert.equal(receivedBody.reason, 'PR closed');
      assert.equal(receivedBody.abandoner, 'human');
      assert.ok(stdout.includes('abandoned'), 'abandon status shown');
    } finally {
      srv.close();
    }
  });

  it('overlay-conflict sends repo_id and unwraps conflict response', async () => {
    let receivedBody = null;
    const srv = makeServer(null);
    srv.setup((req, res) => {
      if (req.url === '/overlay/report_conflict' && req.method === 'POST') {
        let body = '';
        req.on('data', chunk => { body += chunk; });
        req.on('end', () => {
          receivedBody = JSON.parse(body);
          res.writeHead(200, { 'Content-Type': 'application/json' });
          res.end(JSON.stringify({
            status: 'ok',
            data: {
              conflict_id: 'conflict-1',
              has_conflict: true,
              requires_human_review: true,
              overlap_nodes: ['n1'],
              overlap_edges: [],
            },
          }));
        });
      }
    });
    await srv.start();
    try {
      const { stdout } = await execFileAsync(process.execPath, [
        'bin/memfuse.cjs', 'overlay-conflict',
        '--repo', 'symphony-gh',
        '--overlay-a', 'overlay-1',
        '--overlay-b', 'overlay-2',
        '--description', 'same node',
        `--server=http://127.0.0.1:${srv.port()}`,
      ], { cwd });
      assert.equal(receivedBody.repo_id, 'symphony-gh');
      assert.equal(receivedBody.overlay_id_1, 'overlay-1');
      assert.equal(receivedBody.overlay_id_2, 'overlay-2');
      assert.equal(receivedBody.conflict_description, 'same node');
      assert.ok(stdout.includes('has_conflict=true'), 'conflict result shown');
    } finally {
      srv.close();
    }
  });

  it('--help shows new commands', async () => {
    const { stdout } = await execFileAsync(process.execPath, ['bin/memfuse.cjs', '--help'], { cwd });
    assert.ok(stdout.includes('create-fact'), 'help should list create-fact');
    assert.ok(stdout.includes('create-rule'), 'help should list create-rule');
    assert.ok(stdout.includes('mkdir'), 'help should list mkdir');
    assert.ok(stdout.includes('link'), 'help should list link');
    assert.ok(stdout.includes('watches-list'), 'help should list watches-list');
    assert.ok(stdout.includes('resource-export'), 'help should list resource-export');
    assert.ok(stdout.includes('code-symbols-search'), 'help should list code-symbols-search');
    assert.ok(stdout.includes('session-archive'), 'help should list session-archive');
    assert.ok(stdout.includes('repo-manifest'), 'help should list repo-manifest');
    assert.ok(stdout.includes('overlay-propose'), 'help should list overlay-propose');
    assert.ok(!stdout.includes('compact                    Compact session'), 'help should not list removed compact command');
    assert.ok(!stdout.includes('create-annotation'), 'help should not list removed evaluation commands');
    assert.ok(!stdout.includes('signal-quality'), 'help should not list removed evaluation commands');
    assert.ok(!stdout.includes('skill-suggestions'), 'help should not list removed skill evolution commands');
    assert.ok(!stdout.includes('skill-apply-patch'), 'help should not list removed skill evolution commands');
  });
});
