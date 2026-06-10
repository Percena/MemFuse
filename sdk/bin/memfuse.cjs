#!/usr/bin/env node
// MemFuse CLI entry point — thin CJS wrapper around the ESM build.
import('../dist/cli/index.js')
  .then((m) => m.runCli())
  .catch((err) => {
    console.error(`memfuse: ${err && err.message ? err.message : err}`);
    process.exit(1);
  });
