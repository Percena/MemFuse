/**
 * Generic Memory Adapter
 *
 * Platform-neutral adapter for hosts that call the MemFuse lifecycle directly.
 */

import { BaseHostMemoryAdapter } from '../adapter.js';
import type { MemFuseRuntimeClient } from '../runtime-client.js';
import type { HostMemoryAdapterHooks } from '../types.js';

export class GenericMemoryAdapter extends BaseHostMemoryAdapter {
  constructor(client: MemFuseRuntimeClient, hooks: HostMemoryAdapterHooks = {}) {
    super(client, hooks);
  }
}
