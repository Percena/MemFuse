#!/usr/bin/env node
// MemFuse user-prompt-submit hook entry point — thin CJS wrapper around the ESM build.
// Hooks are fail-open: any wrapper-level failure exits 0 so the agent is never blocked.
import('../../dist/hooks/user-prompt-submit.js')
  .then((m) => m.default())
  .catch(() => process.exit(0));
