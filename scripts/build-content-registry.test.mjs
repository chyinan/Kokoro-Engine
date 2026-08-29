// pattern: Functional Core

import { describe, expect, it } from 'vitest';
import {
  OFFICIAL_REGISTRY_IDENTITY,
  OFFICIAL_REGISTRY_URL,
  normalizeTrustSource,
  validateRegistryEntry,
  validateRegistryIndex,
} from './build-content-registry.mjs';

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

  it('validates checksum, archive size, trust label, permissions, and recommendations', () => {
    for (const field of ['sha256', 'archive_size', 'trust']) {
      const invalid = { ...archive, [field]: field === 'archive_size' ? 0 : field === 'trust' ? 'community' : 'not-a-checksum' };
      expect(validateRegistryEntry(invalid).valid).toBe(false);
    }
    expect(validateRegistryEntry({ ...archive, permissions: ['tts', 'system.info'] }).valid).toBe(true);
    expect(validateRegistryEntry({ ...archive, permissions: ['exec.shell'] }).valid).toBe(false);
    expect(validateRegistryEntry({ ...archive, recommendations: { vision: 'yes' } }).valid).toBe(false);
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
});
