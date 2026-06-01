/**
 * MemFuse Memory Lifecycle Runtime Helper
 *
 * Paths aligned with Rust mfs-server Axum routes.
 */

import type { MemFuseRuntimeClient } from './runtime-client.js';
import type { RunMemoryLifecycleInput, RunMemoryLifecycleResult } from './types.js';

export async function runMemoryLifecycle(
  client: MemFuseRuntimeClient,
  input: RunMemoryLifecycleInput,
): Promise<RunMemoryLifecycleResult> {
  const userTurn = await client.appendUserTurn(input.threadId, {
    content: input.userMessage,
    metadata: { resource_id: input.resourceId },
  }, input.userTurnOptions);

  const context = await client.resolveContext({
    user_id: input.userId,
    session_id: input.threadId,
    resource_id: input.resourceId,
    query: input.queryText,
    token_budget: input.budget,
  }, input.resolveContextOptions);

  const assistantTurn = await client.appendAssistantTurn(input.threadId, {
    content: input.assistantMessage,
    metadata: { resource_id: input.resourceId },
  }, input.assistantTurnOptions);

  return { userTurn, context, assistantTurn };
}
