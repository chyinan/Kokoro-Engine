// pattern: Imperative Shell

import { createHash } from 'node:crypto';
import { readFile, readdir, mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { basename, extname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const REGISTRY_SCHEMA_VERSION = 1;
export const REGISTRY_VERSION = 1;
export const OFFICIAL_REGISTRY_URL =
  'https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json';
export const OFFICIAL_PACKAGE_BASE_URL =
  'https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages';
export const OFFICIAL_REGISTRY_IDENTITY = 'github.com/chyinan/Kokoro-Engine/registry-v1';

const ID_PATTERN = /^[a-z0-9](?:[a-z0-9-]{0,62}[a-z0-9])?$/;
const SHA256_PATTERN = /^[a-f0-9]{64}$/;
const TRUST_LABELS = new Set(['official', 'community', 'unverified']);
const CONTENT_TYPES = new Set(['character', 'mod']);
const PERMISSIONS = new Set([
  'tts',
  'vision',
  'memory',
  'mcp',
  'bot',
  'system.info',
  'system.notifications',
  'filesystem.read',
  'clipboard.read',
]);
const BLOCKED_NAMES = new Set([
  '.env',
  '.env.local',
  'id_rsa',
  'id_ed25519',
  'credentials.json',
  'secrets.json',
]);
const NATIVE_EXECUTABLE_EXTENSIONS = new Set([
  '.exe',
  '.dll',
  '.so',
  '.dylib',
  '.bat',
  '.cmd',
  '.ps1',
  '.sh',
]);
const CHARACTER_SCRIPT_EXTENSIONS = new Set(['.js', '.mjs', '.cjs']);

function invalid(message) {
  return { valid: false, error: message };
}

function entrySize(entry) {
  return Number.isInteger(entry.archive_size) ? entry.archive_size : entry.size_bytes;
}

function entryEngineVersion(entry) {
  return typeof entry.engine_version === 'string'
    ? entry.engine_version
    : `>=${entry.min_engine_version ?? '0.0.0'}, <=${entry.max_engine_version ?? '999.999.999'}`;
}

function isEngineRange(value) {
  return value.split('||').every(alternative => alternative.split(',').every(part =>
    /^(?:\s*(?:\^|~|>=|<=|>|<|=)?\s*[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?\s*)$/.test(part),
  ));
}

function archiveName(entry) {
  return `${entry.id}-${entry.version}.zip`;
}

function isValidHttpsUrl(value) {
  if (typeof value !== 'string' || !value.startsWith('https://')) return false;
  try {
    const parsed = new URL(value);
    return parsed.protocol === 'https:' && parsed.username === '' && parsed.password === '';
  } catch {
    return false;
  }
}

function isSafePreviewReference(value) {
  if (typeof value !== 'string' || value.length === 0 || value.length > 512) return false;
  if (value.startsWith('https://')) {
    try {
      const parsed = new URL(value);
      return parsed.protocol === 'https:' && parsed.hostname !== '' && parsed.username === '' && parsed.password === '';
    } catch {
      return false;
    }
  }
  if (value.startsWith('/') || value.startsWith('//') || value.includes('\\') || value.includes(':')) return false;
  return value.split('/').every(segment => segment !== '' && segment !== '.' && segment !== '..');
}

function validateRecommendations(value) {
  if (value === null || typeof value !== 'object' || Array.isArray(value)) return false;
  if (typeof value.vision !== 'boolean' || typeof value.memory !== 'boolean') return false;
  for (const key of ['mcp_servers', 'bot_platforms']) {
    if (!Array.isArray(value[key])
      || new Set(value[key]).size !== value[key].length
      || value[key].some(item => typeof item !== 'string' || !/^[a-z0-9][a-z0-9._-]*$/i.test(item))) {
      return false;
    }
  }
  return true;
}

/** Validate one registry entry without reading from disk. */
export function validateRegistryEntry(entry) {
  if (entry === null || typeof entry !== 'object' || Array.isArray(entry)) return invalid('entry must be an object');
  for (const field of ['content_type', 'id', 'name', 'version', 'author', 'description', 'download_url', 'sha256', 'trust', 'trust_source']) {
    if (typeof entry[field] !== 'string' || entry[field].trim() === '') return invalid(`missing ${field}`);
  }
  if (!CONTENT_TYPES.has(entry.content_type)) return invalid('invalid content_type');
  if (!ID_PATTERN.test(entry.id)) return invalid('invalid id');
  if (!/^[0-9]+\.[0-9]+\.[0-9]+(?:-[0-9A-Za-z.-]+)?(?:\+[0-9A-Za-z.-]+)?$/.test(entry.version)) return invalid('invalid version');
  if (typeof entryEngineVersion(entry) !== 'string' || entryEngineVersion(entry).trim() === '' || !isEngineRange(entryEngineVersion(entry))) return invalid('invalid engine_version');
  if (!isValidHttpsUrl(entry.download_url)) return invalid('download_url must use HTTPS');
  let download;
  try {
    download = new URL(entry.download_url);
  } catch {
    return invalid('invalid download_url');
  }
  if (basename(download.pathname) !== archiveName(entry)) return invalid('download URL basename mismatch');
  if (!Number.isInteger(entrySize(entry)) || entrySize(entry) <= 0) return invalid('invalid archive size');
  if (!SHA256_PATTERN.test(entry.sha256)) return invalid('invalid sha256');
  if (!TRUST_LABELS.has(entry.trust)) return invalid('invalid trust label');
  if (!isValidHttpsUrl(entry.trust_source)) return invalid('invalid trust_source');
  const preview = entry.preview ?? [];
  if (!Array.isArray(preview) || preview.some(value => !isSafePreviewReference(value))) return invalid('invalid preview reference');
  const official = normalizeTrustSource(entry.trust_source);
  if (entry.trust === 'official' && (official.trust !== 'official' || entry.registry_identity !== OFFICIAL_REGISTRY_IDENTITY)) return invalid('official trust requires canonical registry identity');
  if (entry.trust !== 'official' && entry.registry_identity !== null && entry.registry_identity !== undefined) return invalid('non-official entry cannot claim registry identity');
  if (!Array.isArray(entry.permissions)
    || new Set(entry.permissions).size !== entry.permissions.length
    || entry.permissions.some(permission => typeof permission !== 'string' || !PERMISSIONS.has(permission))) return invalid('invalid permissions');
  if (!validateRecommendations(entry.recommendations)) return invalid('invalid recommendations');
  return { valid: true };
}

/** Validate index shape and reject duplicate content IDs. */
export function validateRegistryIndex(index) {
  if (index === null || typeof index !== 'object' || Array.isArray(index)) return invalid('index must be an object');
  if (index.schema_version !== REGISTRY_SCHEMA_VERSION || index.registry_version !== REGISTRY_VERSION) return invalid('unsupported registry version');
  if (!Array.isArray(index.entries) || index.entries.length === 0) return invalid('entries must contain at least one item');
  const seen = new Set();
  for (const entry of index.entries) {
    const result = validateRegistryEntry(entry);
    if (!result.valid) return result;
    const key = `${entry.content_type}:${entry.id}`;
    if (seen.has(key)) return invalid(`duplicate entry ${key}`);
    seen.add(key);
  }
  return { valid: true };
}

/** Resolve trust from the endpoint, ignoring package self-asserted labels. */
export function normalizeTrustSource(source) {
  if (source === OFFICIAL_REGISTRY_URL) {
    return { trust: 'official', trust_source: OFFICIAL_REGISTRY_URL, registry_identity: OFFICIAL_REGISTRY_IDENTITY };
  }
  return { trust: 'community', trust_source: source, registry_identity: null };
}

function crc32(buffer) {
  let crc = 0xffffffff;
  for (const byte of buffer) {
    crc ^= byte;
    for (let bit = 0; bit < 8; bit += 1) crc = (crc >>> 1) ^ (0xedb88320 & -(crc & 1));
  }
  return (crc ^ 0xffffffff) >>> 0;
}

function zipStored(files) {
  const locals = [];
  const centrals = [];
  let offset = 0;
  for (const file of files) {
    const name = Buffer.from(file.name, 'utf8');
    const data = file.data;
    const header = Buffer.alloc(30 + name.length);
    header.writeUInt32LE(0x04034b50, 0);
    header.writeUInt16LE(20, 4);
    header.writeUInt16LE(0, 6);
    header.writeUInt16LE(0, 8);
    header.writeUInt16LE(0, 10);
    header.writeUInt16LE(0, 12);
    header.writeUInt32LE(crc32(data), 14);
    header.writeUInt32LE(data.length, 18);
    header.writeUInt32LE(data.length, 22);
    header.writeUInt16LE(name.length, 26);
    header.writeUInt16LE(0, 28);
    name.copy(header, 30);
    locals.push(header, data);
    const central = Buffer.alloc(46 + name.length);
    central.writeUInt32LE(0x02014b50, 0);
    central.writeUInt16LE(20, 4);
    central.writeUInt16LE(20, 6);
    central.writeUInt16LE(0, 8);
    central.writeUInt16LE(0, 10);
    central.writeUInt16LE(0, 12);
    central.writeUInt16LE(0, 14);
    central.writeUInt32LE(crc32(data), 16);
    central.writeUInt32LE(data.length, 20);
    central.writeUInt32LE(data.length, 24);
    central.writeUInt16LE(name.length, 28);
    central.writeUInt16LE(0, 30);
    central.writeUInt16LE(0, 32);
    central.writeUInt16LE(0, 34);
    central.writeUInt16LE(0, 36);
    central.writeUInt32LE(0, 38);
    central.writeUInt32LE(offset, 42);
    name.copy(central, 46);
    centrals.push(central);
    offset += header.length + data.length;
  }
  const central = Buffer.concat(centrals);
  const eocd = Buffer.alloc(22);
  eocd.writeUInt32LE(0x06054b50, 0);
  eocd.writeUInt16LE(0, 4);
  eocd.writeUInt16LE(0, 6);
  eocd.writeUInt16LE(files.length, 8);
  eocd.writeUInt16LE(files.length, 10);
  eocd.writeUInt32LE(central.length, 12);
  eocd.writeUInt32LE(offset, 16);
  eocd.writeUInt16LE(0, 20);
  return Buffer.concat([...locals, central, eocd]);
}

function readStoredZip(archive) {
  const files = new Map();
  let offset = 0;
  while (offset + 4 <= archive.length && archive.readUInt32LE(offset) === 0x04034b50) {
    if (offset + 30 > archive.length) throw new Error('truncated ZIP local header');
    const method = archive.readUInt16LE(offset + 8);
    const compressedSize = archive.readUInt32LE(offset + 18);
    const nameLength = archive.readUInt16LE(offset + 26);
    const extraLength = archive.readUInt16LE(offset + 28);
    if (method !== 0) throw new Error('registry archives must use deterministic stored entries');
    const nameStart = offset + 30;
    const dataStart = nameStart + nameLength + extraLength;
    const dataEnd = dataStart + compressedSize;
    if (dataEnd > archive.length) throw new Error('truncated ZIP entry');
    const name = archive.subarray(nameStart, nameStart + nameLength).toString('utf8');
    files.set(name, archive.subarray(dataStart, dataEnd));
    offset = dataEnd;
  }
  if (files.size === 0) throw new Error('registry archive contains no files');
  return files;
}

/** Verify generated bytes against the registry metadata and embedded manifest. */
export function verifyRegistryArtifact(entry, archive) {
  if (!Buffer.isBuffer(archive)) return invalid('archive must be a Buffer');
  if (archive.length !== entrySize(entry)) return invalid('archive size mismatch');
  if (createHash('sha256').update(archive).digest('hex') !== entry.sha256) return invalid('archive checksum mismatch');
  try {
    const download = new URL(entry.download_url);
    if (basename(download.pathname) !== archiveName(entry)) return invalid('download URL basename mismatch');
    const files = readStoredZip(archive);
    const manifestName = entry.content_type === 'character' ? 'character.json' : 'mod.json';
    const manifestBytes = files.get(manifestName);
    if (!manifestBytes) return invalid(`archive missing ${manifestName}`);
    const manifest = JSON.parse(manifestBytes.toString('utf8'));
    if (manifest.id !== entry.id || manifest.version !== entry.version || manifest.engine_version !== entry.engine_version) return invalid('manifest metadata mismatch');
  } catch (error) {
    return invalid(error instanceof Error ? error.message : 'invalid archive');
  }
  return { valid: true };
}

async function collectFiles(root, contentType) {
  const result = [];
  async function walk(directory) {
    const entries = await readdir(directory, { withFileTypes: true });
    entries.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
    for (const entry of entries) {
      const fullPath = join(directory, entry.name);
      if (entry.isDirectory()) await walk(fullPath);
      else if (entry.isSymbolicLink() || !entry.isFile()) throw new Error(`registry source contains symlink or non-regular file: ${relative(root, fullPath)}`);
      else {
        const relativePath = relative(root, fullPath).split(sep).join('/');
        const lowerName = entry.name.toLowerCase();
        const extension = extname(lowerName);
        if (BLOCKED_NAMES.has(lowerName)
          || NATIVE_EXECUTABLE_EXTENSIONS.has(extension)
          || (contentType === 'character' && CHARACTER_SCRIPT_EXTENSIONS.has(extension))) {
          throw new Error(`blocked executable or secret resource: ${relativePath}`);
        }
        result.push({ name: relativePath, data: await readFile(fullPath) });
      }
    }
  }
  await walk(root);
  return result;
}

async function readManifest(root, contentType) {
  const manifestName = contentType === 'character' ? 'character.json' : 'mod.json';
  const value = JSON.parse(await readFile(join(root, manifestName), 'utf8'));
  if (typeof value.id !== 'string' || typeof value.version !== 'string' || typeof value.engine_version !== 'string') throw new Error(`invalid ${manifestName}`);
  return value;
}

async function buildArchive(root, outputPath, contentType) {
  const files = await collectFiles(root, contentType);
  files.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
  const archive = zipStored(files);
  await writeFile(outputPath, archive);
  return archive;
}

function registryEntry(manifest, contentType, archive, sourceUrl, packageBaseUrl) {
  const normalizedTrust = normalizeTrustSource(sourceUrl);
  const recommendations = {
    vision: false,
    memory: false,
    mcp_servers: [],
    bot_platforms: [],
    ...(manifest.recommendations ?? {}),
  };
  return {
    content_type: contentType,
    id: manifest.id,
    name: manifest.name,
    version: manifest.version,
    author: manifest.author ?? 'Kokoro Engine',
    description: manifest.description,
    preview: manifest.avatar ? [manifest.avatar] : [],
    engine_version: manifest.engine_version,
    download_url: `${packageBaseUrl}/${manifest.id}-${manifest.version}.zip`,
    archive_size: archive.length,
    sha256: createHash('sha256').update(archive).digest('hex'),
    ...normalizedTrust,
    permissions: manifest.permissions ?? [],
    recommendations,
  };
}

/** Build deterministic archives and publish a validated local index. */
export async function buildContentRegistry({ root = resolve(fileURLToPath(new URL('..', import.meta.url))), sourceUrl = OFFICIAL_REGISTRY_URL } = {}) {
  const registryRoot = join(root, 'registry');
  const packageRoot = join(registryRoot, 'packages');
  const packageBaseUrl = (() => {
    try {
      const url = new URL(sourceUrl);
      url.pathname = url.pathname.replace(/\/v1\/index\.json$/, '/packages');
      return url.toString().replace(/\/$/, '');
    } catch {
      return OFFICIAL_PACKAGE_BASE_URL;
    }
  })();
  await mkdir(packageRoot, { recursive: true });
  const entries = [];
  for (const contentType of ['character', 'mod']) {
    const sourceRoot = join(root, `${contentType === 'character' ? 'characters' : 'mods'}`);
    if (!existsSync(sourceRoot)) continue;
    const children = (await readdir(sourceRoot, { withFileTypes: true })).filter(entry => entry.isDirectory()).sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
    for (const child of children) {
      const source = join(sourceRoot, child.name);
      if (contentType === 'character' && child.name.toLowerCase() === 'template') continue;
      const manifest = await readManifest(source, contentType);
      const archivePath = join(packageRoot, `${manifest.id}-${manifest.version}.zip`);
      const archive = await buildArchive(source, archivePath, contentType);
      const entry = registryEntry(manifest, contentType, archive, sourceUrl, packageBaseUrl);
      const artifact = verifyRegistryArtifact(entry, archive);
      if (!artifact.valid) throw new Error(`generated archive ${child.name} is invalid: ${artifact.error}`);
      entries.push(entry);
    }
  }
  entries.sort((left, right) => {
    const a = `${left.content_type}:${left.id}`;
    const b = `${right.content_type}:${right.id}`;
    return a < b ? -1 : a > b ? 1 : 0;
  });
  const index = { schema_version: REGISTRY_SCHEMA_VERSION, registry_version: REGISTRY_VERSION, generated_at: '1970-01-01T00:00:00.000Z', entries };
  const validation = validateRegistryIndex(index);
  if (!validation.valid) throw new Error(`generated registry is invalid: ${validation.error}`);
  await mkdir(join(registryRoot, 'v1'), { recursive: true });
  await writeFile(join(registryRoot, 'v1', 'index.json'), `${JSON.stringify(index, null, 2)}\n`, 'utf8');
  return index;
}

const invokedPath = process.argv[1] ? pathToFileURL(resolve(process.argv[1])).href : null;
if (invokedPath === import.meta.url) {
  buildContentRegistry().then(index => {
    process.stdout.write(`built ${index.entries.length} registry entries\n`);
  }).catch(error => {
    process.stderr.write(`${error.message}\n`);
    process.exitCode = 1;
  });
}
