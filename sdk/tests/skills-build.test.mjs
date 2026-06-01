import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { access, readdir } from 'node:fs/promises';

async function skillDirs(root) {
  const entries = await readdir(new URL(root, import.meta.url), { withFileTypes: true });
  return entries
    .filter((entry) => entry.isDirectory())
    .map((entry) => entry.name)
    .sort();
}

describe('skills build output', () => {
  it('does not leave dist skill directories without matching source skills', async () => {
    const sourceSkills = await skillDirs('../src/skills/');
    const distSkills = await skillDirs('../dist/skills/');

    assert.deepEqual(distSkills, sourceSkills);
  });

  it('copies bundled MemFuse skill references to dist', async () => {
    await access(new URL('../dist/skills/memfuse/references/commands.md', import.meta.url));
  });
});
