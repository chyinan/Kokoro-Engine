// pattern: Functional Core

import { createHash } from 'node:crypto';
import { mkdtemp, mkdir, readFile, readdir, symlink, writeFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { afterEach, describe, expect, it } from 'vitest';
import {
  buildContentRegistry,
  OFFICIAL_REGISTRY_IDENTITY,
  OFFICIAL_REGISTRY_URL,
  normalizeTrustSource,
  verifyRegistryArtifact,
  validateContentFileNames,
  validateContentFileSizes,
  validateRegistryEntry,
  validateRegistryIndex,
} from './build-content-registry.mjs';

const temporaryRoots = [];

function storedZip(files) {
  const chunks = [];
  for (const file of files) {
    const name = Buffer.from(file.name, 'utf8');
    const data = Buffer.from(file.data);
    const header = Buffer.alloc(30 + name.length);
    header.writeUInt32LE(0x04034b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt32LE(data.length, 18);
    header.writeUInt32LE(data.length, 22);
    header.writeUInt16LE(name.length, 26);
    name.copy(header, 30);
    chunks.push(header, data);
  }
  return Buffer.concat(chunks);
}

function artifactFor(files, overrides = {}) {
  const bytes = storedZip(files);
  const manifest = files.find((file) => file.name === 'character.json' || file.name === 'mod.json');
  const contentType = manifest?.name === 'mod.json' ? 'mod' : 'character';
  const parsed = JSON.parse(Buffer.from(manifest.data).toString('utf8'));
  const base = {
    ...archive,
    content_type: contentType,
    id: parsed.id,
    version: parsed.version,
    engine_version: parsed.engine_version,
    download_url: `${OFFICIAL_REGISTRY_URL}/../packages/${parsed.id}-${parsed.version}.zip`,
    archive_size: bytes.length,
    sha256: createHash('sha256').update(bytes).digest('hex'),
    preview: [],
    ...overrides,
  };
  return { entry: base, bytes };
}

function characterManifest(overrides = {}) {
  return {
    schema_version: 1,
    engine_version: '>=0.3.1, <0.4.0',
    id: 'artifact-character',
    version: '1.0.0',
    name: 'Artifact Character',
    description: 'A character',
    author: 'Kokoro',
    license: 'MIT',
    persona: 'Be helpful.',
    greeting: 'Hello.',
    ...overrides,
  };
}

afterEach(async () => {
  await Promise.all(temporaryRoots.splice(0).map((root) => rm(root, { recursive: true, force: true })));
});

const archive = {
  content_type: 'character',
  id: 'kokoro',
  name: 'Kokoro',
  version: '1.0.0',
  author: 'Kokoro Engine',
  description: 'A warm daily companion.',
  engine_version: '>=0.3.1, <0.4.0',
  download_url: `${OFFICIAL_REGISTRY_URL}/../packages/kokoro-1.0.0.zip`,
  archive_size: 1234,
  sha256: 'a'.repeat(64),
  trust: 'official',
  trust_source: OFFICIAL_REGISTRY_URL,
  registry_identity: OFFICIAL_REGISTRY_IDENTITY,
  permissions: [],
  recommendations: { vision: false, memory: true, mcp_servers: [], bot_platforms: [] },
};

function validIndex(entries = [archive]) {
  return { schema_version: 1, registry_version: 1, entries };
}

describe('content registry contract', () => {
  it('accepts character and MOD entry content types and compatibility metadata', () => {
    expect(validateRegistryEntry(archive)).toEqual({ valid: true });
    expect(validateRegistryEntry({ ...archive, content_type: 'mod' })).toEqual({ valid: true });
    expect(validateRegistryEntry({ ...archive, engine_version: 'not semver' }).valid).toBe(false);
  });

  it('requires HTTPS URLs with a matching archive basename', () => {
    expect(validateRegistryEntry(archive)).toEqual({ valid: true });
    expect(validateRegistryEntry({ ...archive, download_url: 'http://example.test/kokoro-1.0.0.zip' }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, download_url: `${OFFICIAL_REGISTRY_URL}/../packages/other.zip` }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, download_url: `${OFFICIAL_REGISTRY_URL}/../packages/kokoro-1.0.0.tar.gz` }).valid).toBe(false);
  });

  it('accepts only safe preview references', () => {
    expect(validateRegistryEntry({ ...archive, preview: ['https://cdn.example.test/kokoro.webp'] })).toEqual({ valid: true });
    expect(validateRegistryEntry({ ...archive, preview: ['assets/kokoro.webp'] })).toEqual({ valid: true });
    expect(validateRegistryEntry({ ...archive, preview: ['javascript:alert(1)'] }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, preview: ['http://cdn.example.test/kokoro.webp'] }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, preview: ['../outside.webp'] }).valid).toBe(false);
  });

  it('validates checksum, archive size, trust label, permissions, and recommendations', () => {
    for (const field of ['sha256', 'archive_size', 'trust']) {
      const invalid = { ...archive, [field]: field === 'archive_size' ? 0 : field === 'trust' ? 'community' : 'not-a-checksum' };
      expect(validateRegistryEntry(invalid).valid).toBe(false);
    }
    expect(validateRegistryEntry({ ...archive, permissions: ['tts', 'system.info'] }).valid).toBe(true);
    expect(validateRegistryEntry({ ...archive, permissions: ['tts', 'tts'] }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, permissions: ['exec.shell'] }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, recommendations: { vision: 'yes' } }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, recommendations: { ...archive.recommendations, mcp_servers: ['calendar', 'calendar'] } }).valid).toBe(false);
  });

  it('rejects duplicate IDs within a content type', () => {
    const duplicate = { ...archive, version: '1.0.1', download_url: `${OFFICIAL_REGISTRY_URL}/../packages/kokoro-1.0.1.zip` };
    expect(validateRegistryIndex(validIndex([archive, duplicate])).valid).toBe(false);
    expect(validateRegistryIndex(validIndex([archive, { ...archive, content_type: 'mod', id: 'kokoro' }])).valid).toBe(true);
  });

  it('normalizes trust source and never self-asserts official for custom endpoints', () => {
    expect(normalizeTrustSource(OFFICIAL_REGISTRY_URL)).toEqual({
      trust: 'official',
      trust_source: OFFICIAL_REGISTRY_URL,
      registry_identity: OFFICIAL_REGISTRY_IDENTITY,
    });
    expect(normalizeTrustSource('https://mirror.example.test/index.json')).toEqual({
      trust: 'community',
      trust_source: 'https://mirror.example.test/index.json',
      registry_identity: null,
    });
    expect(validateRegistryEntry({ ...archive, trust: 'official', trust_source: 'https://mirror.example.test/index.json' }).valid).toBe(false);
  });

  it('allows documented MOD JavaScript but rejects character scripts', async () => {
    const root = await mkdtemp(join(tmpdir(), 'kokoro-registry-policy-'));
    temporaryRoots.push(root);
    await mkdir(join(root, 'mods', 'script-mod'), { recursive: true });
    await writeFile(join(root, 'mods', 'script-mod', 'mod.json'), JSON.stringify({
      id: 'script-mod', name: 'Script Mod', version: '1.0.0', description: 'A documented MOD script',
      engine_version: '>=0.3.1, <0.4.0', scripts: ['main.js'], permissions: [],
    }));
    await writeFile(join(root, 'mods', 'script-mod', 'main.js'), 'export function activate() {}\n');
    const index = await buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' });
    expect(index.entries).toHaveLength(1);
    expect(index.entries[0].content_type).toBe('mod');

    await mkdir(join(root, 'characters', 'script-character'), { recursive: true });
    await writeFile(join(root, 'characters', 'script-character', 'character.json'), JSON.stringify({
      schema_version: 1, id: 'script-character', version: '1.0.0', name: 'Script Character',
      description: 'A character', author: 'Kokoro', license: 'MIT', engine_version: '>=0.3.1, <0.4.0',
      persona: 'Be helpful.', greeting: 'Hello.',
    }));
    await writeFile(join(root, 'characters', 'script-character', 'script.js'), 'alert(1);\n');
    await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/blocked|unsupported|executable/i);
  });

  it('rejects a symlink that would include content outside the package', async () => {
    const root = await mkdtemp(join(tmpdir(), 'kokoro-registry-symlink-'));
    temporaryRoots.push(root);
    const packageRoot = join(root, 'mods', 'linked-mod');
    await mkdir(packageRoot, { recursive: true });
    await writeFile(join(root, 'outside.txt'), 'must not be archived');
    await writeFile(join(packageRoot, 'mod.json'), JSON.stringify({
      id: 'linked-mod', name: 'Linked Mod', version: '1.0.0', description: 'A MOD',
      engine_version: '>=0.3.1, <0.4.0', permissions: [],
    }));
    await symlink(join(root, 'outside.txt'), join(packageRoot, 'linked.txt'), 'file');
    await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/symlink|link|outside/i);
  });

  it('requires a root license-named file for character packages', async () => {
    const root = await mkdtemp(join(tmpdir(), 'kokoro-registry-license-'));
    temporaryRoots.push(root);
    const packageRoot = join(root, 'characters', 'unlicensed');
    await mkdir(packageRoot, { recursive: true });
    await writeFile(join(packageRoot, 'character.json'), JSON.stringify({
      schema_version: 1, id: 'unlicensed', version: '1.0.0', name: 'Unlicensed',
      description: 'A character', author: 'Kokoro', license: 'MIT', engine_version: '>=0.3.1, <0.4.0',
      persona: 'Be helpful.', greeting: 'Hello.',
    }));
    await mkdir(join(packageRoot, 'licenses'));
    await writeFile(join(packageRoot, 'licenses', 'MIT.txt'), 'MIT');

    await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/root license/i);
  });

  it('does not publish the creator template directory by default', async () => {
    const root = await mkdtemp(join(tmpdir(), 'kokoro-registry-template-'));
    temporaryRoots.push(root);
    await mkdir(join(root, 'characters', 'template'), { recursive: true });
    await writeFile(join(root, 'characters', 'template', 'character.json'), '{ this is documentation scaffolding, not a package }');
    await mkdir(join(root, 'characters', 'kokoro'), { recursive: true });
    await writeFile(join(root, 'characters', 'kokoro', 'character.json'), JSON.stringify({
      schema_version: 1, id: 'kokoro', version: '1.0.0', name: 'Kokoro',
      description: 'Built-in character', author: 'Kokoro', license: 'MIT', engine_version: '>=0.3.1, <0.4.0',
      persona: 'Kokoro', greeting: 'Hello',
    }));
    await writeFile(join(root, 'characters', 'kokoro', 'LICENSE.md'), 'MIT');
    const index = await buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' });
    expect(index.entries.map((entry) => entry.id)).toEqual(['kokoro']);
  });

  it('validates manifest identity and semantic ranges before deriving an archive path', async () => {
    const cases = [
      { id: '../escape', version: '1.0.0', engine_version: '>=0.3.1, <0.4.0' },
      { id: 'leading-zero', version: '01.0.0', engine_version: '>=0.3.1, <0.4.0' },
      { id: 'bad-prerelease', version: '1.0.0-01', engine_version: '>=0.3.1, <0.4.0' },
      { id: 'bad-build', version: '1.0.0+build..1', engine_version: '>=0.3.1, <0.4.0' },
      { id: 'bad-range', version: '1.0.0', engine_version: '>=0.3.1-01, <0.4.0' },
    ];
    for (const [index, values] of cases.entries()) {
      const root = await mkdtemp(join(tmpdir(), `kokoro-registry-manifest-${index}-`));
      temporaryRoots.push(root);
      const packageRoot = join(root, 'characters', 'candidate');
      await mkdir(packageRoot, { recursive: true });
      await writeFile(join(packageRoot, 'character.json'), JSON.stringify({
        schema_version: 1, ...values, name: 'Candidate', description: 'A character',
        author: 'Kokoro', license: 'MIT', persona: 'Be helpful.', greeting: 'Hello.',
      }));
      await writeFile(join(packageRoot, 'LICENSE.md'), 'MIT');

      await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
        .rejects.toThrow(/invalid .*?(id|version|engine_version)|semantic|range/i);
      expect((await readdir(join(root, 'registry'), { withFileTypes: true }).catch(() => [])).some((entry) => entry.name === 'escape-1.0.0.zip')).toBe(false);
    }
  });

  it('keeps archive file policy aligned with character and MOD installers', async () => {
    expect(validateContentFileNames(['character.json', 'cues.json', 'LICENSE.md', 'live2d/model.model3.json'], 'character').valid).toBe(true);
    expect(validateContentFileNames(['character.json', 'LICENSE.md', 'payload.bin'], 'character').valid).toBe(false);
    expect(validateContentFileNames(['character.json', 'LICENSE.md', 'extra.json'], 'character').valid).toBe(false);
    expect(validateContentFileNames(['mod.json', 'main.js', 'assets/theme.css'], 'mod').valid).toBe(true);
    expect(validateContentFileNames(['mod.json', 'payload.bin'], 'mod').valid).toBe(false);
    expect(validateContentFileNames(['mod.json', 'unsafe/../main.js'], 'mod').valid).toBe(false);
    expect(validateContentFileNames(['mod.json', 'main.js', 'MAIN.JS'], 'mod').valid).toBe(false);
  });

  it('enforces installer-equivalent character and MOD size limits before archiving', () => {
    const characterAtFileLimit = Array.from({ length: 2048 }, (_, index) => ({
      name: `assets/${index}.png`,
      size: 1,
    }));
    expect(validateContentFileSizes(characterAtFileLimit, 'character')).toEqual({ valid: true });
    expect(validateContentFileSizes([...characterAtFileLimit, { name: 'assets/overflow.png', size: 1 }], 'character').valid).toBe(false);
    expect(validateContentFileSizes([{ name: 'assets/full.png', size: 128 * 1024 * 1024 }], 'character')).toEqual({ valid: true });
    expect(validateContentFileSizes([{ name: 'assets/overflow.png', size: 128 * 1024 * 1024 + 1 }], 'character').valid).toBe(false);

    expect(validateContentFileSizes([{ name: 'main.js', size: 10 * 1024 * 1024 }], 'mod')).toEqual({ valid: true });
    expect(validateContentFileSizes([{ name: 'main.js', size: 10 * 1024 * 1024 + 1 }], 'mod').valid).toBe(false);
    expect(validateContentFileSizes(Array.from({ length: 5 }, (_, index) => ({
      name: `assets/${index}.txt`,
      size: 10 * 1024 * 1024,
    })), 'mod')).toEqual({ valid: true });
    expect(validateContentFileSizes([
      ...Array.from({ length: 4 }, (_, index) => ({ name: `assets/${index}.txt`, size: 10 * 1024 * 1024 })),
      { name: 'assets/overflow.txt', size: 10 * 1024 * 1024 + 1 },
    ], 'mod').valid).toBe(false);
  });

  it('requires the full CharacterTemplateManifest required field set', async () => {
    const root = await mkdtemp(join(tmpdir(), 'kokoro-registry-character-schema-'));
    temporaryRoots.push(root);
    const packageRoot = join(root, 'characters', 'candidate');
    await mkdir(packageRoot, { recursive: true });
    await writeFile(join(packageRoot, 'LICENSE.md'), 'MIT');
    const manifest = {
      schema_version: 1,
      engine_version: '>=0.3.1, <0.4.0',
      id: 'candidate',
      version: '1.0.0',
      name: 'Candidate',
      description: 'A character',
      author: 'Kokoro',
      license: 'MIT',
      persona: 'Be helpful.',
      greeting: 'Hello.',
    };

    for (const field of ['schema_version', 'engine_version', 'id', 'version', 'name', 'description', 'author', 'license', 'persona', 'greeting']) {
      const candidate = { ...manifest };
      delete candidate[field];
      await writeFile(join(packageRoot, 'character.json'), JSON.stringify(candidate));
      await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
        .rejects.toThrow(new RegExp(field));
    }

    await writeFile(join(packageRoot, 'character.json'), JSON.stringify({ ...manifest, schema_version: 2 }));
    await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/schema_version|schema/i);
  });

  it('requires relative manifest previews to resolve to an image in the package', async () => {
    const root = await mkdtemp(join(tmpdir(), 'kokoro-registry-preview-'));
    temporaryRoots.push(root);
    const packageRoot = join(root, 'characters', 'preview');
    await mkdir(packageRoot, { recursive: true });
    await writeFile(join(packageRoot, 'character.json'), JSON.stringify({
      schema_version: 1, id: 'preview', version: '1.0.0', name: 'Preview', description: 'A character',
      author: 'Kokoro', license: 'MIT', engine_version: '>=0.3.1, <0.4.0', persona: 'Be helpful.',
      greeting: 'Hello.', avatar: 'assets/avatar.png',
    }));
    await writeFile(join(packageRoot, 'LICENSE.md'), 'MIT');
    await expect(buildContentRegistry({ root, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/preview|avatar|missing|package/i);
  });

  it('verifies the complete archive content policy, including duplicate and unsupported paths', () => {
    const manifest = Buffer.from(JSON.stringify(characterManifest()));
    const unsupported = artifactFor([
      { name: 'character.json', data: manifest },
      { name: 'LICENSE.md', data: 'MIT' },
      { name: 'scripts/install.js', data: 'alert(1)' },
    ]);
    expect(verifyRegistryArtifact(unsupported.entry, unsupported.bytes).valid).toBe(false);

    const duplicate = artifactFor([
      { name: 'character.json', data: manifest },
      { name: 'LICENSE.md', data: 'MIT' },
      { name: 'license.md', data: 'shadow' },
    ]);
    expect(verifyRegistryArtifact(duplicate.entry, duplicate.bytes)).toMatchObject({
      valid: false,
      error: expect.stringMatching(/duplicate/i),
    });
  });

  it('rejects artifact manifests with unknown CharacterTemplateManifest fields', () => {
    const manifest = Buffer.from(JSON.stringify(characterManifest({ unexpected: true })));
    const candidate = artifactFor([
      { name: 'character.json', data: manifest },
      { name: 'LICENSE.md', data: 'MIT' },
    ]);

    expect(verifyRegistryArtifact(candidate.entry, candidate.bytes)).toMatchObject({
      valid: false,
      error: expect.stringMatching(/unknown|manifest/i),
    });

    const nested = artifactFor([
      {
        name: 'character.json',
        data: JSON.stringify(characterManifest({ assets: { cue_profile: 'cues.json', unexpected: true } })),
      },
      { name: 'LICENSE.md', data: 'MIT' },
      { name: 'cues.json', data: '{}' },
    ]);
    expect(verifyRegistryArtifact(nested.entry, nested.bytes)).toMatchObject({
      valid: false,
      error: expect.stringMatching(/unknown|manifest/i),
    });
  });

  it('applies MOD per-file limits while verifying generated artifacts', () => {
    const manifest = Buffer.from(JSON.stringify({
      id: 'large-mod', name: 'Large MOD', version: '1.0.0', description: 'A MOD',
      engine_version: '>=0.3.1, <0.4.0', permissions: [],
    }));
    const candidate = artifactFor([
      { name: 'mod.json', data: manifest },
      { name: 'main.js', data: Buffer.alloc(10 * 1024 * 1024 + 1, 0x61) },
    ]);

    expect(verifyRegistryArtifact(candidate.entry, candidate.bytes)).toMatchObject({
      valid: false,
      error: expect.stringMatching(/10 MiB|size|limit/i),
    });
  });

  it('refuses to write registry output through symlinked registry directories or archives', async () => {
    const makeMod = async (root) => {
      const packageRoot = join(root, 'mods', 'safe-mod');
      await mkdir(packageRoot, { recursive: true });
      await writeFile(join(packageRoot, 'mod.json'), JSON.stringify({
        id: 'safe-mod', name: 'Safe Mod', version: '1.0.0', description: 'A MOD',
        engine_version: '>=0.3.1, <0.4.0', permissions: [],
      }));
    };

    const registryLinkRoot = await mkdtemp(join(tmpdir(), 'kokoro-registry-output-link-'));
    temporaryRoots.push(registryLinkRoot);
    await makeMod(registryLinkRoot);
    const registryOutside = await mkdtemp(join(tmpdir(), 'kokoro-registry-output-outside-'));
    temporaryRoots.push(registryOutside);
    await symlink(registryOutside, join(registryLinkRoot, 'registry'), 'dir');
    await expect(buildContentRegistry({ root: registryLinkRoot, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/symlink|regular directory|output/i);

    const packageLinkRoot = await mkdtemp(join(tmpdir(), 'kokoro-package-output-link-'));
    temporaryRoots.push(packageLinkRoot);
    await makeMod(packageLinkRoot);
    await mkdir(join(packageLinkRoot, 'registry', 'packages'), { recursive: true });
    const packageOutside = await mkdtemp(join(tmpdir(), 'kokoro-package-output-outside-'));
    temporaryRoots.push(packageOutside);
    await writeFile(join(packageOutside, 'archive.zip'), 'must not be overwritten');
    await symlink(join(packageOutside, 'archive.zip'), join(packageLinkRoot, 'registry', 'packages', 'safe-mod-1.0.0.zip'), 'file');
    await expect(buildContentRegistry({ root: packageLinkRoot, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/symlink|regular file|output/i);

    const v1LinkRoot = await mkdtemp(join(tmpdir(), 'kokoro-v1-output-link-'));
    temporaryRoots.push(v1LinkRoot);
    await makeMod(v1LinkRoot);
    await mkdir(join(v1LinkRoot, 'registry', 'packages'), { recursive: true });
    const v1Outside = await mkdtemp(join(tmpdir(), 'kokoro-v1-output-outside-'));
    temporaryRoots.push(v1Outside);
    await symlink(v1Outside, join(v1LinkRoot, 'registry', 'v1'), 'dir');
    await expect(buildContentRegistry({ root: v1LinkRoot, sourceUrl: 'https://mirror.example.test/registry/v1/index.json' }))
      .rejects.toThrow(/symlink|regular directory|output/i);
  });
});
