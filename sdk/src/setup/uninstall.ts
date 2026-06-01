/**
 * MemFuse Uninstall
 *
 * Removes MCP server, hooks, and skills configuration.
 * Uses CLI commands preferred (claude mcp remove / codex mcp remove),
 * JSON file fallback when CLI unavailable.
 * Skills are removed from platform skill directories.
 */

import { readFileSync, writeFileSync, existsSync, rmSync } from 'node:fs';
import { join } from 'node:path';
import { execSync } from 'node:child_process';
import { uninstallSkill, SKILL_NAMES } from '../skills/loader.js';

interface UninstallOptions {
  platform?: 'claude-code' | 'codex' | 'both';
  projectDir?: string;
}

/** Try to remove MCP server via CLI command, return true if succeeded */
function tryMcpCliRemove(cliCmd: string, serverName: string): boolean {
  try {
    execSync(`${cliCmd} ${serverName}`, { stdio: 'pipe', timeout: 10000 });
    return true;
  } catch {
    return false;
  }
}

function removeFromJson(file: string): void {
  if (!existsSync(file)) return;

  let config: Record<string, unknown>;
  try {
    config = JSON.parse(readFileSync(file, 'utf-8'));
  } catch { return; }

  // Remove MCP server entry
  if (config.mcpServers && typeof config.mcpServers === 'object') {
    delete (config.mcpServers as Record<string, unknown>).memfuse;
    if (Object.keys(config.mcpServers as Record<string, unknown>).length === 0) delete config.mcpServers;
  }

  // Remove memfuse hooks (Claude Code format: array)
  if (Array.isArray(config.hooks)) {
    config.hooks = config.hooks.filter((h: Record<string, unknown>) => {
      const cmd = (h.cmd || []) as string[];
      const cmdStr = cmd.join(' ');
      return !cmdStr.includes('memfuse') && !cmdStr.includes('MemFuse');
    });
    if ((config.hooks as unknown[]).length === 0) delete config.hooks;
  }

  // Remove memfuse hooks (Codex format: nested object)
  if (typeof config.hooks === 'object' && !Array.isArray(config.hooks)) {
    for (const eventName of Object.keys(config.hooks as Record<string, unknown>)) {
      const groups = (config.hooks as Record<string, unknown[]>)[eventName] as Record<string, unknown>[];
      (config.hooks as Record<string, unknown[]>)[eventName] = groups.filter(group => {
        const groupHooks = (group.hooks || []) as Record<string, unknown>[];
        group.hooks = groupHooks.filter(h => {
          const cmd = String(h.command || '');
          return !cmd.includes('memfuse') && !cmd.includes('MemFuse');
        });
        return (group.hooks as unknown[]).length > 0;
      });
      if (((config.hooks as Record<string, unknown[]>)[eventName] as unknown[]).length === 0) {
        delete (config.hooks as Record<string, unknown[]>)[eventName];
      }
    }
    if (Object.keys(config.hooks as Record<string, unknown>).length === 0) delete config.hooks;
  }

  writeFileSync(file, JSON.stringify(config, null, 2) + '\n', 'utf-8');
  console.log(`  ✓ Removed memfuse configuration from ${file}`);
}

export async function runUninstall(args: string[]): Promise<void> {
  const options: UninstallOptions = {
    platform: 'both',
    projectDir: process.cwd(),
  };

  for (const arg of args) {
    if (arg.startsWith('--platform=')) options.platform = arg.split('=')[1] as UninstallOptions['platform'];
    else if (arg.startsWith('--project-dir=')) options.projectDir = join(process.cwd(), arg.split('=')[1]);
    else { console.error(`Unknown argument: ${arg}`); process.exit(1); }
  }

  console.log('MemFuse Uninstall');
  console.log(`  Platform:    ${options.platform}`);
  console.log(`  Project dir: ${options.projectDir}`);
  console.log('');

  if (options.platform === 'claude-code' || options.platform === 'both') {
    console.log('=== Uninstalling from Claude Code ===');

    // MCP — try CLI first, fallback to JSON
    const cliRemoved = tryMcpCliRemove('claude mcp remove', 'memfuse');
    if (cliRemoved) {
      console.log('  ✓ MCP server removed via `claude mcp remove`');
    } else {
      const settingsFile = join(options.projectDir!, '.claude', 'settings.local.json');
      removeFromJson(settingsFile);
    }

    // Hooks — remove from JSON
    const settingsFile = join(options.projectDir!, '.claude', 'settings.local.json');
    removeFromJson(settingsFile);

    // Skills — remove from .claude/skills/
    const skillsDir = join(options.projectDir!, '.claude', 'skills');
    for (const name of SKILL_NAMES) {
      const removed = uninstallSkill(name, skillsDir);
      if (removed) console.log(`  ✓ Skill '${name}' removed from ${skillsDir}`);
    }

    // Plugin manifest — remove .claude-plugin/
    const pluginDir = join(options.projectDir!, '.claude-plugin');
    if (existsSync(pluginDir)) {
      rmSync(pluginDir, { recursive: true, force: true });
      console.log(`  ✓ Plugin manifest removed from ${pluginDir}`);
    }

    console.log('=== Claude Code uninstall complete ===');
  }

  if (options.platform === 'codex' || options.platform === 'both') {
    console.log('=== Uninstalling from Codex ===');

    // MCP — try CLI first, fallback to JSON
    const cliRemoved = tryMcpCliRemove('codex mcp remove', 'memfuse');
    if (cliRemoved) {
      console.log('  ✓ MCP server removed via `codex mcp remove`');
    } else {
      const mcpFile = join(options.projectDir!, '.codex', 'mcp.json');
      removeFromJson(mcpFile);
    }

    // Hooks — remove from JSON
    const hooksFile = join(options.projectDir!, '.codex', 'hooks.json');
    removeFromJson(hooksFile);

    // Skills — remove from .codex/skills/
    const skillsDir = join(options.projectDir!, '.codex', 'skills');
    for (const name of SKILL_NAMES) {
      const removed = uninstallSkill(name, skillsDir);
      if (removed) console.log(`  ✓ Skill '${name}' removed from ${skillsDir}`);
    }

    // Plugin manifest — remove .codex-plugin/
    const pluginDir = join(options.projectDir!, '.codex-plugin');
    if (existsSync(pluginDir)) {
      rmSync(pluginDir, { recursive: true, force: true });
      console.log(`  ✓ Plugin manifest removed from ${pluginDir}`);
    }

    console.log('=== Codex uninstall complete ===');
  }

  console.log('');
  console.log('MemFuse uninstallation finished!');
}