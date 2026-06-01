#!/usr/bin/env node
/**
 * MemFuse Setup Hook
 *
 * Health check for server availability.
 * Runs at session startup to verify the memory system is online.
 * Phase 0: single-server routing via checkHealth().
 */

import { checkHealth, readStdin, EXIT_OK, isCliEntryPoint } from './platform-utils.js';

export default async function run(): Promise<void> {
  try {
    const rawInput = await readStdin();
    // We don't need to parse input for setup — just check health

    const healthy = await checkHealth();

    if (!healthy) {
      process.stderr.write('⚠️ MemFuse: Memory service is offline. MemFuse MCP tools will not be available.\n');
      process.stderr.write('   Start it with: `memfuse service start`\n');
      process.stderr.write('   Development server: `./run-server.sh`\n');
      process.stderr.write('   Override endpoint with MEMFUSE_SERVER_URL if needed.\n');
    }

    process.exit(EXIT_OK);
  } catch (err) {
    process.stderr.write(`MemFuse Setup check error: ${err instanceof Error ? err.message : String(err)}\n`);
    process.exit(EXIT_OK);
  }
}

// Auto-invoke only when this ESM file is executed directly.
if (isCliEntryPoint(import.meta.url)) {
  run().catch(() => process.exit(EXIT_OK));
}
