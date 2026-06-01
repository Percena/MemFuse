import { describe, it } from 'node:test';
import assert from 'node:assert/strict';

import {
  createJsonRpcFrameParser,
  extractFactIdFromMcpResult,
  listMissingEvidence,
  mcpToolSucceeded,
} from './e2e/claude-code-harness.mjs';

describe('Claude Code E2E harness helpers', () => {
  it('treats MCP application-level isError results as failed tool calls', () => {
    assert.equal(mcpToolSucceeded({ ok: true, result: { isError: true } }), false);
    assert.equal(mcpToolSucceeded({ ok: true, result: { content: [{ type: 'text', text: 'ok' }] } }), true);
    assert.equal(mcpToolSucceeded({ ok: false, error: 'transport failed' }), false);
  });

  it('extracts fact ids from MCP text and structured results', () => {
    assert.equal(
      extractFactIdFromMcpResult({ content: [{ type: 'text', text: 'Fact created.\nID: fact_12345\nsubject status: ok' }] }),
      'fact_12345',
    );
    assert.equal(
      extractFactIdFromMcpResult({ fact_id: 'fact_abcde' }),
      'fact_abcde',
    );
    assert.equal(
      extractFactIdFromMcpResult({ content: [{ type: 'text', text: 'No active facts found.' }] }),
      '',
    );
  });

  it('lists missing evidence keys whose values are not true', () => {
    assert.deepEqual(
      listMissingEvidence({ a: true, b: false, c: '', d: 0, e: 'ok' }, ['a', 'b', 'c', 'd', 'e']),
      ['b', 'c', 'd'],
    );
  });

  it('parses Content-Length frames by bytes when JSON contains Unicode', () => {
    const messages = [];
    const parser = createJsonRpcFrameParser((message) => messages.push(message));
    const body = JSON.stringify({ jsonrpc: '2.0', id: 1, result: { text: '📍 ✓ →' } });
    const frame = Buffer.from(`Content-Length: ${Buffer.byteLength(body)}\r\n\r\n${body}`);

    parser.push(frame.subarray(0, 12));
    parser.push(frame.subarray(12));

    assert.deepEqual(messages, [{ jsonrpc: '2.0', id: 1, result: { text: '📍 ✓ →' } }]);
  });

  it('parses newline-delimited JSON-RPC frames used by the MCP stdio transport', () => {
    const messages = [];
    const parser = createJsonRpcFrameParser((message) => messages.push(message));

    parser.push(Buffer.from('{"jsonrpc":"2.0","id":1,'));
    parser.push(Buffer.from('"result":{"ok":true}}\n{"jsonrpc":"2.0","id":2,"result":{"ok":false}}\n'));

    assert.deepEqual(messages, [
      { jsonrpc: '2.0', id: 1, result: { ok: true } },
      { jsonrpc: '2.0', id: 2, result: { ok: false } },
    ]);
  });
});
