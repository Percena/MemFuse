/**
 * MemFuse packaging tests — npm bin metadata and user-facing install commands.
 */

import { describe, it } from 'node:test';
import assert from 'node:assert/strict';
import { readFile, stat } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const sdkRoot = dirname(dirname(fileURLToPath(import.meta.url)));
const repoRoot = dirname(sdkRoot);

describe('npm package metadata', () => {
  it('declares canonical executable bin targets', async () => {
    const pkg = JSON.parse(await readFile(join(sdkRoot, 'package.json'), 'utf8'));
    const expectedBins = {
      'memfuse-mcp': 'bin/memfuse-mcp.cjs',
      'memfuse-setup': 'bin/memfuse-setup.cjs',
      memfuse: 'bin/memfuse.cjs',
    };

    assert.deepEqual(pkg.bin, expectedBins);

    for (const [command, target] of Object.entries(expectedBins)) {
      const binPath = join(sdkRoot, target);
      const contents = await readFile(binPath, 'utf8');
      const mode = (await stat(binPath)).mode;

      assert.ok(!target.startsWith('./'), `${command} bin target should not need npm path normalization`);
      assert.match(contents, /^#!\/usr\/bin\/env node/, `${command} should be directly executable by npm`);
      if (process.platform !== 'win32') {
        assert.ok((mode & 0o111) !== 0, `${command} wrapper should be executable`);
      }
    }
  });
});

describe('README packaging commands', () => {
  it('documents scoped-package npx commands for clean directories', async () => {
    const readme = await readFile(join(sdkRoot, 'README.md'), 'utf8');

    assert.ok(readme.includes('npx --package=@percena/memfuse memfuse '));
    assert.ok(readme.includes('npx --package=@percena/memfuse memfuse-setup '));
    assert.ok(readme.includes('npx --package=@percena/memfuse memfuse-mcp'));
  });

  it('does not document bare npx commands for package bins', async () => {
    for (const relativePath of ['README.md', 'sdk/README.md']) {
      const readme = await readFile(join(repoRoot, relativePath), 'utf8');

      assert.ok(!readme.includes('npx memfuse '), `${relativePath}: bare npx memfuse resolves the unscoped package name`);
      assert.ok(!readme.includes('npx memfuse-setup '), `${relativePath}: bare npx memfuse-setup resolves the unscoped package name`);
      assert.ok(!readme.includes('npx memfuse-mcp'), `${relativePath}: bare npx memfuse-mcp resolves the unscoped package name`);
    }
  });
});
