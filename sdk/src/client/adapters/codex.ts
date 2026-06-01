/**
 * Codex Memory Adapter
 */

import { BaseHostMemoryAdapter } from '../adapter.js';
import type { MemFuseRuntimeClient } from '../runtime-client.js';
import type { PrepareReadInput, StartTurnInput } from '../types.js';

export class CodexMemoryAdapter extends BaseHostMemoryAdapter {
  constructor(client: MemFuseRuntimeClient) {
    super(client);
  }

  async prepareSessionContext(input: StartTurnInput): Promise<string> {
    const prepared = await this.prepareTurn(input);
    return prepared.renderedText;
  }

  async prepareReadHint(input: PrepareReadInput): Promise<string> {
    const prepared = await this.prepareRead(input);
    return prepared.renderedText;
  }
}
