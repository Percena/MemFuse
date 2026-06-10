#!/usr/bin/env node
// MemFuse MCP stdio server entry point — thin CJS wrapper around the ESM build.
import('../dist/mcp/server.js')
  .then((m) => m.startServer())
  .catch((err) => {
    console.error(`memfuse-mcp: ${err && err.message ? err.message : err}`);
    process.exit(1);
  });
