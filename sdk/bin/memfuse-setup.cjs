#!/usr/bin/env node
// MemFuse setup entry point — `memfuse-setup install|uninstall [flags]`.
const [, , command, ...args] = process.argv;

async function main() {
  if (command === 'install' || command === undefined) {
    const { runSetup } = await import('../dist/setup/install.js');
    await runSetup(args);
  } else if (command === 'uninstall') {
    const { runUninstall } = await import('../dist/setup/uninstall.js');
    await runUninstall(args);
  } else {
    console.error(`Unknown command: ${command}\nUsage: memfuse-setup install|uninstall [--platform=claude-code|codex|both] [--project-dir=...] [--server-url=...] [--user-id=...]`);
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(`memfuse-setup: ${err && err.message ? err.message : err}`);
  process.exit(1);
});
