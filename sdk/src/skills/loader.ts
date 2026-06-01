/**
 * MemFuse Skills Loader
 *
 * Reads SKILL.md files for MCP tool usage and skill installation.
 * Installation now uses standard platform skill directories:
 *   - Claude Code: .claude/skills/memfuse/SKILL.md
 *   - Codex:       .codex/skills/memfuse/SKILL.md
 *
 * Marker-block injection into CLAUDE.md/AGENTS.md has been removed
 * in favor of the native skill directory convention.
 */

import { cpSync, existsSync, mkdirSync, readFileSync, rmSync } from 'node:fs';
import { join, dirname } from 'node:path';

const SKILLS_DIR = join(dirname(new URL(import.meta.url).pathname.replace(/^\/([A-Z]:)/, '$1')), '..', 'skills');

export const SKILL_NAMES = ['memfuse'] as const;

/** Read a skill's SKILL.md content */
export function loadSkill(name: string): string {
  const filePath = join(skillSourceDir(name), 'SKILL.md');
  if (!existsSync(filePath)) {
    throw new Error(`Skill not found: ${name} (expected at ${filePath})`);
  }
  return readFileSync(filePath, 'utf-8');
}

function skillSourceDir(name: string): string {
  return join(SKILLS_DIR, name);
}

/**
 * Copy a full skill directory to a target platform skills directory.
 * Creates the directory if it doesn't exist.
 */
export function installSkill(name: string, targetSkillsDir: string): string {
  const sourceDir = skillSourceDir(name);
  if (!existsSync(join(sourceDir, 'SKILL.md'))) {
    throw new Error(`Skill not found: ${name} (expected at ${join(sourceDir, 'SKILL.md')})`);
  }
  const skillDir = join(targetSkillsDir, name);
  const targetPath = join(skillDir, 'SKILL.md');

  mkdirSync(targetSkillsDir, { recursive: true });
  rmSync(skillDir, { recursive: true, force: true });
  cpSync(sourceDir, skillDir, { recursive: true, force: true });

  return targetPath;
}

/**
 * Remove a skill directory from a target platform skills directory.
 */
export function uninstallSkill(name: string, targetSkillsDir: string): boolean {
  const skillDir = join(targetSkillsDir, name);
  if (!existsSync(skillDir)) return false;

  rmSync(skillDir, { recursive: true, force: true });
  return true;
}
