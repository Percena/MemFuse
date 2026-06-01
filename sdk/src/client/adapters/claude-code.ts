/**
 * Claude Code Memory Adapter
 */

import { BaseHostMemoryAdapter } from '../adapter.js';
import type { MemFuseRuntimeClient } from '../runtime-client.js';

export class ClaudeCodeMemoryAdapter extends BaseHostMemoryAdapter {
  constructor(client: MemFuseRuntimeClient) {
    super(client);
  }
}