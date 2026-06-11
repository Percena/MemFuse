#!/usr/bin/env node
/**
 * MemFuse PostToolUse Hook
 *
 * Automatically captures observations from tool executions.
 * Claude Code: all tools (Edit/Write/Read/Bash/MCP etc.)
 * Codex: only Bash tool calls.
 * Privacy: <private>...</private> blocks are stripped before storage.
 * B1: multi-endpoint routing via callBackend + CanvasRouter with offline degradation.
 */

import { adaptInput, callBackend, readStdin, truncate, sanitizeMemoryText, isDegradableError, EXIT_OK, PATHS, loadConfig, isCliEntryPoint, router } from './platform-utils.js';

const config = loadConfig();

// Low-value tool patterns to skip
const SKIP_PATTERNS = [
  /^Bash:\s*(ls|pwd|date|echo|cat|head|tail|tree|which|whoami|uname)\b/,
  /^Bash:\s*git\s+(log|status|diff|branch|remote)\b/,
  /^Glob$/,
  /^Notification$/,
  /^TodoWrite$/,
];

function shouldSkip(toolName: string, toolInput: string): boolean {
  const key = `${toolName}:${toolInput.substring(0, 50)}`;
  return SKIP_PATTERNS.some(pattern => pattern.test(key));
}

// Extract file path from Edit/Write/Read tool input
function extractFilePath(toolName: string, toolInput: string): string | null {
  if (toolName === 'Edit' || toolName === 'Write' || toolName === 'Read') {
    try {
      const parsed = JSON.parse(toolInput);
      if (parsed && parsed.file_path) return parsed.file_path;
    } catch (_) { /* not JSON */ }
  }
  return null;
}

/**
 * Compute structured metadata from a tool execution.
 * Returns a metadata object with tool_type, summary, affected_files, outcome, and key_result.
 */
export function computeMetadata(toolName: string, toolInput: string, toolOutput: string): Record<string, unknown> {
  // Classify tool type
  let toolType = 'Other';
  if (toolName === 'Read') toolType = 'Read';
  else if (toolName === 'Bash') toolType = 'Bash';
  else if (toolName === 'Edit' || toolName === 'Write' || toolName === 'MultiEdit') toolType = 'Edit';
  else if (toolName === 'Grep' || toolName === 'Glob' || toolName === 'Search') toolType = 'Search';

  // Summary: first 120 chars of content
  const summary = toolOutput.substring(0, 120).replace(/\n/g, ' ').trim();

  // Extract affected files from tool_input JSON
  const affectedFiles: string[] = [];
  try {
    const parsed = JSON.parse(toolInput);
    if (parsed.file_path) affectedFiles.push(parsed.file_path);
    if (parsed.file_paths && Array.isArray(parsed.file_paths)) {
      affectedFiles.push(...parsed.file_paths);
    }
  } catch (_) { /* not JSON — check for file paths in string */ }
  // Also try regex for file paths in non-JSON input
  if (affectedFiles.length === 0) {
    const filePattern = /(?:\/[\w.-]+)+\.[\w]+/g;
    const matches = toolInput.match(filePattern);
    if (matches) affectedFiles.push(...matches.slice(0, 5));
  }

  // Detect outcome (error vs success)
  const isError = /error|Error|ERROR|failed|FAILED|panic|fatal/i.test(toolOutput);

  // Key result: test counts, line numbers, etc.
  let keyResult: string | null = null;
  const testMatch = toolOutput.match(/(\d+)\s+(?:passing|failed|tests?|specs?)/i);
  if (testMatch) keyResult = testMatch[0];
  const lineMatch = toolOutput.match(/(?:line|Line)\s+(\d+)/);
  if (!keyResult && lineMatch) keyResult = lineMatch[0];

  const result: Record<string, unknown> = { tool_type: toolType };
  if (summary) result.summary = summary;
  if (affectedFiles.length > 0) result.affected_files = affectedFiles;
  result.outcome = isError ? 'error' : 'success';
  if (keyResult) result.key_result = keyResult;

  return result;
}

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    if (!rawInput.trim()) { process.exit(EXIT_OK); return; }

    const input = adaptInput(JSON.parse(rawInput));
    const userId = input.user_id || config.userId;
    const sessionId = input.session_id;
    const toolName = input.tool_name;
    const toolInput = input.tool_input;
    const toolOutput = input.tool_output;

    // Filter low-value observations
    if (shouldSkip(toolName, toolInput)) {
      process.exit(EXIT_OK);
      return;
    }

    // Strip private blocks and redact common token patterns before storage.
    const safeInput = sanitizeMemoryText(toolInput);
    const safeOutput = sanitizeMemoryText(toolOutput);

    // Truncate large inputs/outputs
    const truncatedInput = truncate(safeInput, 5000);
    const truncatedOutput = truncate(safeOutput, 10000);

    // Determine content summary with file path extraction
    let content = '';
    const filePath = extractFilePath(toolName, safeInput);
    if (toolName === 'Edit') {
      content = filePath ? `Modified file: ${filePath}` : 'Modified file: extracted from Edit operation';
    } else if (toolName === 'Write') {
      content = filePath ? `Created/overwritten file: ${filePath}` : 'Created file: extracted from Write operation';
    } else if (toolName === 'Read') {
      content = filePath ? `Read file: ${filePath}` : 'Read file: extracted from Read operation';
    } else if (toolName === 'Bash') {
      content = `Executed command: ${truncate(String(safeInput), 100)}`;
    } else if (toolName.startsWith('mcp__')) {
      content = `MCP tool call: ${toolName}`;
    } else {
      content = `${toolName} tool execution`;
    }

    // Determine source trust level (memory pollution tracking)
    const isExternal = toolName.startsWith('mcp__')
      || toolName === 'WebSearch'
      || toolName === 'WebFetch'
      || /^Bash:\s*(curl|wget|http)\b/.test(`${toolName}:${safeInput.substring(0, 50)}`);
    const sourceTrust = isExternal ? 'external' : 'internal';

    // Store observation — POST /sessions/{sessionId}/observations
    const observePath = `${PATHS.OBSERVE}/${sessionId || 'default'}/observations`;
    const metadata = computeMetadata(toolName, truncatedInput, truncatedOutput);
    try {
      await callBackend('POST', observePath, {
        tool_name: toolName,
        tool_input: truncatedInput,
        tool_output: truncatedOutput,
        content,
        platform: input._platform,
        source_trust: sourceTrust,
        metadata,
      }, router);
    } catch (err) {
      if (isDegradableError(err)) {
        process.stderr.write(`MemFuse observation write degraded: ${err instanceof Error ? err.message : String(err)}\n`);
      } else {
        process.stderr.write(`MemFuse observation write error: ${err instanceof Error ? err.message : String(err)}\n`);
      }
    }

    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse PostToolUse error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
