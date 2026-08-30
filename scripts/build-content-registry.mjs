// pattern: Imperative Shell

import { createHash } from 'node:crypto';
import { lstat, readFile, readdir, mkdir, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { basename, dirname, extname, join, relative, resolve, sep } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

export const REGISTRY_SCHEMA_VERSION = 1;
export const REGISTRY_VERSION = 1;
export const OFFICIAL_REGISTRY_URL =
  'https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json';
export const OFFICIAL_PACKAGE_BASE_URL =
  'https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages';
export const OFFICIAL_REGISTRY_IDENTITY = 'github.com/chyinan/Kokoro-Engine/registry-v1';
export const CHARACTER_SCHEMA_VERSION = 1;
export const MAX_CHARACTER_FILE_COUNT = 2048;
export const MAX_CHARACTER_TOTAL_BYTES = 128 * 1024 * 1024;
export const MAX_MOD_FILE_BYTES = 10 * 1024 * 1024;
export const MAX_MOD_TOTAL_BYTES = 50 * 1024 * 1024;

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
const MOD_ALLOWED_EXTENSIONS = new Set([
  '.html', '.js', '.css', '.json', '.png', '.jpg', '.jpeg', '.webp', '.svg', '.gif',
  '.woff', '.woff2', '.ttf', '.otf', '.txt', '.md',
]);
const CHARACTER_ALLOWED_SUFFIXES = [
  '.png', '.jpg', '.jpeg', '.webp', '.wav', '.ogg', '.mp3', '.moc3', '.model3.json',
  '.motion3.json', '.physics3.json', '.pose3.json', '.exp3.json', '.userdata3.json',
];
const MAX_SEMVER_NUMBER = 18446744073709551615n;
const SEMVER_IDENTIFIER = /^[0-9A-Za-z-]+$/;
const CHARACTER_MANIFEST_FIELDS = new Set([
  'schema_version', 'engine_version', 'id', 'version', 'name', 'description', 'author',
  'license', 'locale', 'avatar', 'persona', 'greeting', 'example_dialogue', 'assets',
  'runtime', 'recommendations',
]);
const CHARACTER_ASSET_FIELDS = new Set(['live2d_model', 'background', 'cue_profile']);
const CHARACTER_RUNTIME_FIELDS = new Set([
  'live2d_model', 'background', 'cue_profile', 'tts', 'response_language', 'proactive_enabled',
]);
const CHARACTER_TTS_FIELDS = new Set([
  'enabled', 'provider_type', 'provider_id', 'local_preset', 'voice', 'speed', 'pitch',
]);
const CHARACTER_RECOMMENDATION_FIELDS = new Set(['vision', 'memory', 'mcp_servers', 'bot_platforms']);

function invalid(message) {
  return { valid: false, error: message };
}

function isObject(value) {
  return value !== null && typeof value === 'object' && !Array.isArray(value);
}

function hasOnlyFields(value, fields, label) {
  if (!isObject(value)) return invalid(`${label} must be an object`);
  const unknown = Object.keys(value).find((field) => !fields.has(field));
  return unknown ? invalid(`${label} contains unknown field: ${unknown}`) : { valid: true };
}

function validateOptionalField(value, field, predicate, label) {
  if (value === undefined || value === null) return { valid: true };
  return predicate(value) ? { valid: true } : invalid(`invalid ${label} ${field}`);
}

function validateCharacterManifestValue(value) {
  const shape = hasOnlyFields(value, CHARACTER_MANIFEST_FIELDS, 'character manifest');
  if (!shape.valid) return shape;
  if (value.schema_version !== CHARACTER_SCHEMA_VERSION) return invalid('invalid character manifest schema_version');
  for (const field of ['engine_version', 'id', 'version', 'name', 'description', 'author', 'license', 'persona', 'greeting']) {
    if (typeof value[field] !== 'string' || value[field].trim() === '') return invalid(`invalid character manifest ${field}`);
  }
  if (!ID_PATTERN.test(value.id)) return invalid('invalid character manifest id');
  if (!isValidSemVer(value.version)) return invalid('invalid character manifest version');
  if (!isValidEngineRange(value.engine_version)) return invalid('invalid character manifest engine_version');
  for (const field of ['locale', 'avatar', 'example_dialogue']) {
    const result = validateOptionalField(value[field], field, (candidate) => typeof candidate === 'string', 'character manifest');
    if (!result.valid) return result;
  }
  if (value.assets !== undefined && value.assets !== null) {
    const assets = hasOnlyFields(value.assets, CHARACTER_ASSET_FIELDS, 'character manifest assets');
    if (!assets.valid) return assets;
    for (const field of CHARACTER_ASSET_FIELDS) {
      const result = validateOptionalField(value.assets[field], field, (candidate) => typeof candidate === 'string', 'character manifest assets');
      if (!result.valid) return result;
    }
  }
  if (value.runtime !== undefined && value.runtime !== null) {
    const runtime = hasOnlyFields(value.runtime, CHARACTER_RUNTIME_FIELDS, 'character manifest runtime');
    if (!runtime.valid) return runtime;
    for (const field of ['live2d_model', 'background', 'cue_profile', 'response_language']) {
      const result = validateOptionalField(value.runtime[field], field, (candidate) => typeof candidate === 'string', 'character manifest runtime');
      if (!result.valid) return result;
    }
    const proactive = validateOptionalField(value.runtime.proactive_enabled, 'proactive_enabled', (candidate) => typeof candidate === 'boolean', 'character manifest runtime');
    if (!proactive.valid) return proactive;
    if (value.runtime.tts !== undefined && value.runtime.tts !== null) {
      const tts = hasOnlyFields(value.runtime.tts, CHARACTER_TTS_FIELDS, 'character manifest runtime.tts');
      if (!tts.valid) return tts;
      for (const field of ['provider_type', 'provider_id', 'local_preset', 'voice']) {
        const result = validateOptionalField(value.runtime.tts[field], field, (candidate) => typeof candidate === 'string', 'character manifest runtime.tts');
        if (!result.valid) return result;
      }
      for (const field of ['enabled']) {
        const result = validateOptionalField(value.runtime.tts[field], field, (candidate) => typeof candidate === 'boolean', 'character manifest runtime.tts');
        if (!result.valid) return result;
      }
      for (const field of ['speed', 'pitch']) {
        const result = validateOptionalField(value.runtime.tts[field], field, (candidate) => typeof candidate === 'number' && Number.isFinite(candidate), 'character manifest runtime.tts');
        if (!result.valid) return result;
      }
    }
  }
  if (value.recommendations !== undefined && value.recommendations !== null) {
    const recommendations = hasOnlyFields(value.recommendations, CHARACTER_RECOMMENDATION_FIELDS, 'character manifest recommendations');
    if (!recommendations.valid) return recommendations;
    for (const field of ['vision', 'memory']) {
      const result = validateOptionalField(value.recommendations[field], field, (candidate) => typeof candidate === 'boolean', 'character manifest recommendations');
      if (!result.valid) return result;
    }
    for (const field of ['mcp_servers', 'bot_platforms']) {
      const result = validateOptionalField(value.recommendations[field], field, (candidate) => Array.isArray(candidate) && candidate.every((item) => typeof item === 'string'), 'character manifest recommendations');
      if (!result.valid) return result;
    }
  }
  return { valid: true };
}

function validateModManifestValue(value) {
  if (!isObject(value)) return invalid('mod manifest must be an object');
  for (const field of ['id', 'name', 'version', 'description']) {
    if (typeof value[field] !== 'string' || value[field].trim() === '') return invalid(`invalid mod manifest ${field}`);
  }
  if (!ID_PATTERN.test(value.id)) return invalid('invalid mod manifest id');
  if (!isValidSemVer(value.version)) return invalid('invalid mod manifest version');
  if (value.engine_version !== undefined && value.engine_version !== null && !isValidEngineRange(value.engine_version)) return invalid('invalid mod manifest engine_version');
  for (const field of ['layout', 'theme', 'entry', 'ui_entry']) {
    if (value[field] !== undefined && value[field] !== null && typeof value[field] !== 'string') return invalid(`invalid mod manifest ${field}`);
    if (typeof value[field] === 'string' && !isSafePackageRelativePath(value[field])) return invalid(`unsafe mod manifest ${field}`);
  }
  if (value.components !== undefined && value.components !== null) {
    if (!isObject(value.components) || Object.entries(value.components).some(([slot, path]) => typeof slot !== 'string' || typeof path !== 'string' || !isSafePackageRelativePath(path))) return invalid('invalid mod manifest components');
  }
  for (const field of ['scripts', 'permissions']) {
    if (value[field] !== undefined && value[field] !== null && (!Array.isArray(value[field]) || value[field].some((item) => typeof item !== 'string'))) return invalid(`invalid mod manifest ${field}`);
  }
  if (value.permissions?.some((permission) => !PERMISSIONS.has(permission))) return invalid('invalid mod manifest permissions');
  if (value.capabilities !== undefined && value.capabilities !== null) {
    if (!Array.isArray(value.capabilities)) return invalid('invalid mod manifest capabilities');
    for (const capability of value.capabilities) {
      if (!isObject(capability) || typeof capability.name !== 'string' || (capability.risk !== undefined && capability.risk !== null && typeof capability.risk !== 'string') || (capability.requires_confirmation !== undefined && typeof capability.requires_confirmation !== 'boolean')) return invalid('invalid mod manifest capability');
    }
  }
  return { valid: true };
}

function entrySize(entry) {
  return Number.isInteger(entry.archive_size) ? entry.archive_size : entry.size_bytes;
}

function entryEngineVersion(entry) {
  return typeof entry.engine_version === 'string'
    ? entry.engine_version
    : `>=${entry.min_engine_version ?? '0.0.0'}, <=${entry.max_engine_version ?? '999.999.999'}`;
}

function parseSemVer(value) {
  if (typeof value !== 'string') return null;
  const match = /^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)(?:-([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?(?:\+([0-9A-Za-z-]+(?:\.[0-9A-Za-z-]+)*))?$/.exec(value);
  if (!match) return null;
  const core = match.slice(1, 4).map(identifier => BigInt(identifier));
  if (core.some(number => number > MAX_SEMVER_NUMBER)) return null;
  const prerelease = match[4]?.split('.') ?? [];
  if (prerelease.some(identifier => !SEMVER_IDENTIFIER.test(identifier)
    || (/^[0-9]+$/.test(identifier) && (identifier.length > 1 && identifier.startsWith('0'))))) return null;
  const build = match[5]?.split('.') ?? [];
  if (build.some(identifier => !SEMVER_IDENTIFIER.test(identifier))) return null;
  return { core, prerelease, build };
}

export function isValidSemVer(value) {
  return parseSemVer(value) !== null;
}

function isValidRangeComparator(part) {
  const match = /^(?:\^|~|>=|<=|>|<|=)?\s*(.*)$/.exec(part.trim());
  if (!match || match[1] === '') return false;
  const value = match[1];
  if (value === '*' || value.toLowerCase() === 'x') return true;
  if (/^(?:\d+|[xX])(?:\.(?:\d+|[xX]))?(?:\.(?:\d+|[xX]))?$/.test(value)) {
    const identifiers = value.split('.');
    const wildcardIndex = identifiers.findIndex(identifier => identifier.toLowerCase() === 'x');
    if (wildcardIndex >= 0) {
      if (identifiers.slice(wildcardIndex).some(identifier => identifier.toLowerCase() !== 'x')) return false;
      return identifiers.slice(0, wildcardIndex).every(identifier => /^(0|[1-9][0-9]*)$/.test(identifier)
        && BigInt(identifier) <= MAX_SEMVER_NUMBER);
    }
    if (identifiers.length < 3) {
      return identifiers.every(identifier => /^(0|[1-9][0-9]*)$/.test(identifier)
        && BigInt(identifier) <= MAX_SEMVER_NUMBER);
    }
    return parseSemVer(value) !== null;
  }
  return parseSemVer(value) !== null;
}

/** Validate the subset of npm-style ranges accepted by Rust semver::VersionReq. */
export function isValidEngineRange(value) {
  if (typeof value !== 'string' || value.trim() === '') return false;
  return value.split('||').every(alternative => {
    const trimmed = alternative.trim();
    if (trimmed === '') return false;
    const hyphen = /^(\S+)\s+-\s+(\S+)$/.exec(trimmed);
    if (hyphen) return isValidSemVer(hyphen[1]) && isValidSemVer(hyphen[2]);
    return trimmed.split(',').every(part => isValidRangeComparator(part));
  });
}

function isSafePackageRelativePath(value) {
  if (typeof value !== 'string' || value.length === 0 || value.includes('\\') || value.includes('\0') || value.includes(':')) return false;
  if (value.startsWith('/') || value.startsWith('//')) return false;
  return value.split('/').every(segment => segment !== '' && segment !== '.' && segment !== '..');
}

function isRootLicenseName(value) {
  const lower = basename(value).toLowerCase();
  return !value.includes('/') && (lower === 'license' || lower.startsWith('license.'));
}

function isSupportedCharacterFile(value) {
  const lower = value.toLowerCase();
  if (lower === 'character.json' || lower === 'cues.json' || basename(lower) === 'license' || basename(lower).startsWith('license.')) return true;
  return CHARACTER_ALLOWED_SUFFIXES.some(suffix => lower.endsWith(suffix));
}

function isSupportedModFile(value) {
  return MOD_ALLOWED_EXTENSIONS.has(extname(value.toLowerCase()));
}

/** Validate archive names before writing bytes, including case-insensitive collisions. */
export function validateContentFileNames(names, contentType) {
  const seen = new Set();
  for (const name of names) {
    if (!isSafePackageRelativePath(name)) return invalid(`unsafe package path: ${name}`);
    const key = name.toLowerCase();
    if (seen.has(key)) return invalid(`case-insensitive duplicate package path: ${name}`);
    seen.add(key);
    if (contentType === 'character' ? !isSupportedCharacterFile(name) : !isSupportedModFile(name)) {
      return invalid(`unsupported ${contentType} package content: ${name}`);
    }
  }
  return { valid: true };
}

/** Validate source file sizes against the corresponding package installer. */
export function validateContentFileSizes(files, contentType) {
  if (!Array.isArray(files)) return invalid('package files must be an array');
  const isCharacter = contentType === 'character';
  const fileLimit = isCharacter ? MAX_CHARACTER_FILE_COUNT : Number.POSITIVE_INFINITY;
  const perFileLimit = isCharacter ? Number.POSITIVE_INFINITY : MAX_MOD_FILE_BYTES;
  const totalLimit = isCharacter ? MAX_CHARACTER_TOTAL_BYTES : MAX_MOD_TOTAL_BYTES;
  if (files.length > fileLimit) return invalid(`package exceeds ${contentType} file count limit`);

  let total = 0;
  for (const file of files) {
    const size = Number.isSafeInteger(file?.size)
      ? file.size
      : Number.isSafeInteger(file?.data?.byteLength)
        ? file.data.byteLength
        : NaN;
    if (!Number.isSafeInteger(size) || size < 0) return invalid(`invalid ${contentType} file size: ${file?.name ?? '<unknown>'}`);
    if (size > perFileLimit) return invalid(`${contentType} file exceeds the 10 MiB limit: ${file?.name ?? '<unknown>'}`);
    total += size;
    if (!Number.isSafeInteger(total) || total > totalLimit) return invalid(`${contentType} package exceeds the ${totalLimit / (1024 * 1024)} MiB total size limit`);
  }
  return { valid: true };
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
  if (typeof value !== 'string' || value.length === 0 || value.length > 512 || /\s/.test(value)) return false;
  if (value.startsWith('https://')) {
    try {
      const parsed = new URL(value);
      return parsed.protocol === 'https:' && parsed.hostname !== '' && parsed.username === '' && parsed.password === '';
    } catch {
      return false;
    }
  }
  if (value.startsWith('/') || value.startsWith('//') || value.includes('\\') || value.includes(':') || value.includes('%')) return false;
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
  if (!isValidSemVer(entry.version)) return invalid('invalid version');
  if (typeof entryEngineVersion(entry) !== 'string' || entryEngineVersion(entry).trim() === '' || !isValidEngineRange(entryEngineVersion(entry))) return invalid('invalid engine_version');
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
  const canonicalNames = new Set();
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
    const canonicalName = name.replaceAll('\\', '/').replace(/\/+$/, '').toLowerCase();
    if (canonicalNames.has(canonicalName)) throw new Error(`registry archive contains case-insensitive duplicate path: ${name}`);
    canonicalNames.add(canonicalName);
    files.set(name, archive.subarray(dataStart, dataEnd));
    offset = dataEnd;
  }
  if (files.size === 0) throw new Error('registry archive contains no files');
  return files;
}

/** Verify generated bytes against the registry metadata and embedded manifest. */
export function verifyRegistryArtifact(entry, archive) {
  if (!Buffer.isBuffer(archive)) return invalid('archive must be a Buffer');
  const entryValidation = validateRegistryEntry(entry);
  if (!entryValidation.valid) return entryValidation;
  if (archive.length !== entrySize(entry)) return invalid('archive size mismatch');
  if (createHash('sha256').update(archive).digest('hex') !== entry.sha256) return invalid('archive checksum mismatch');
  try {
    const download = new URL(entry.download_url);
    if (basename(download.pathname) !== archiveName(entry)) return invalid('download URL basename mismatch');
    const files = readStoredZip(archive);
    const fileEntries = [...files.entries()].map(([name, data]) => ({ name, data, size: data.length }));
    const filePolicy = validateContentFileNames(fileEntries.map((file) => file.name), entry.content_type);
    if (!filePolicy.valid) return filePolicy;
    const sizePolicy = validateContentFileSizes(fileEntries, entry.content_type);
    if (!sizePolicy.valid) return sizePolicy;
    for (const file of fileEntries) {
      const lowerName = basename(file.name).toLowerCase();
      const extension = extname(lowerName);
      if (BLOCKED_NAMES.has(lowerName) || NATIVE_EXECUTABLE_EXTENSIONS.has(extension)
        || (entry.content_type === 'character' && CHARACTER_SCRIPT_EXTENSIONS.has(extension))) {
        return invalid(`blocked executable or secret resource: ${file.name}`);
      }
    }
    const manifestName = entry.content_type === 'character' ? 'character.json' : 'mod.json';
    const manifestBytes = files.get(manifestName);
    if (!manifestBytes) return invalid(`archive missing ${manifestName}`);
    if (entry.content_type === 'character' && ![...files.keys()].some(isRootLicenseName)) {
      return invalid('character package must contain a root license-named file');
    }
    for (const preview of entry.preview ?? []) {
      if (!preview.startsWith('https://') && !files.has(preview)) {
        return invalid(`preview reference is missing from archive: ${preview}`);
      }
    }
    const manifest = JSON.parse(manifestBytes.toString('utf8'));
    const manifestValidation = entry.content_type === 'character'
      ? validateCharacterManifestValue(manifest)
      : validateModManifestValue(manifest);
    if (!manifestValidation.valid) return manifestValidation;
    validateManifestReferences(manifest, [...files.keys()]);
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
        result.push({ name: relativePath, data: await readFile(fullPath) });
      }
    }
  }
  await walk(root);
  for (const file of result) {
    const lowerName = basename(file.name).toLowerCase();
    const extension = extname(lowerName);
    if (BLOCKED_NAMES.has(lowerName) || NATIVE_EXECUTABLE_EXTENSIONS.has(extension)
      || (contentType === 'character' && CHARACTER_SCRIPT_EXTENSIONS.has(extension))) {
      throw new Error(`blocked executable or secret resource: ${file.name}`);
    }
  }
  if (contentType === 'character' && !result.some(file => isRootLicenseName(file.name))) {
    throw new Error('character package must contain a root license-named file');
  }
  const policy = validateContentFileNames(result.map(file => file.name), contentType);
  if (!policy.valid) throw new Error(policy.error);
  const sizePolicy = validateContentFileSizes(result, contentType);
  if (!sizePolicy.valid) throw new Error(sizePolicy.error);
  return result;
}

async function readManifest(root, contentType) {
  const manifestName = contentType === 'character' ? 'character.json' : 'mod.json';
  const value = JSON.parse(await readFile(join(root, manifestName), 'utf8'));
  if (value === null || typeof value !== 'object' || Array.isArray(value)) {
    throw new Error(`invalid ${manifestName} object`);
  }
  const validation = contentType === 'character'
    ? validateCharacterManifestValue(value)
    : validateModManifestValue(value);
  if (!validation.valid) throw new Error(validation.error);
  if (typeof value.engine_version !== 'string' || !isValidEngineRange(value.engine_version)) throw new Error(`invalid ${manifestName} engine_version`);
  return value;
}

function validateManifestReferences(manifest, files) {
  const available = new Set(files);
  const image = value => typeof value === 'string' && /\.(?:png|jpe?g|webp)$/i.test(value);
  const references = [
    ['avatar', manifest.avatar, image],
    ['assets.live2d_model', manifest.assets?.live2d_model, value => /\.model3\.json$/i.test(value)],
    ['assets.background', manifest.assets?.background, image],
    ['assets.cue_profile', manifest.assets?.cue_profile, value => /\.json$/i.test(value)],
  ];
  for (const [label, value, extensionCheck] of references) {
    if (value === undefined || value === null) continue;
    if (!isSafePackageRelativePath(value) || !extensionCheck(value) || !available.has(value)) {
      throw new Error(`invalid or missing ${label} package reference: ${value}`);
    }
  }
}

async function buildArchive(root, contentType, manifest) {
  const files = await collectFiles(root, contentType);
  validateManifestReferences(manifest, files.map(file => file.name));
  files.sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
  return zipStored(files);
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

async function ensureSafeDirectory(path, label) {
  const target = resolve(path);
  const missing = [];
  let current = target;
  while (true) {
    try {
      const metadata = await lstat(current);
      if (metadata.isSymbolicLink() || !metadata.isDirectory()) {
        throw new Error(`${label} contains a symlink or non-directory path: ${current}`);
      }
      break;
    } catch (error) {
      if (error?.code !== 'ENOENT') throw error;
      missing.push(current);
      const parent = dirname(current);
      if (parent === current) throw new Error(`${label} has no safe parent: ${current}`);
      current = parent;
    }
  }
  for (const directory of missing.reverse()) await mkdir(directory);
}

async function writeSafeFile(path, data, label) {
  await ensureSafeDirectory(dirname(path), `${label} parent`);
  try {
    const metadata = await lstat(path);
    if (metadata.isSymbolicLink() || !metadata.isFile()) {
      throw new Error(`${label} is not a regular file: ${path}`);
    }
  } catch (error) {
    if (error?.code !== 'ENOENT') throw error;
  }
  await writeFile(path, data);
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
  await ensureSafeDirectory(packageRoot, 'registry package output');
  const entries = [];
  const outputNames = new Set();
  for (const contentType of ['character', 'mod']) {
    const sourceRoot = join(root, `${contentType === 'character' ? 'characters' : 'mods'}`);
    if (!existsSync(sourceRoot)) continue;
    const sourceMetadata = await lstat(sourceRoot);
    if (sourceMetadata.isSymbolicLink() || !sourceMetadata.isDirectory()) throw new Error(`registry source is not a regular directory: ${sourceRoot}`);
    const sourceEntries = await readdir(sourceRoot, { withFileTypes: true });
    for (const sourceEntry of sourceEntries) {
      if (sourceEntry.isSymbolicLink() || (!sourceEntry.isDirectory() && sourceEntry.name.toLowerCase() !== '.gitkeep')) {
        throw new Error(`registry source contains symlink or non-directory entry: ${sourceEntry.name}`);
      }
    }
    const children = sourceEntries.filter(entry => entry.isDirectory()).sort((left, right) => left.name < right.name ? -1 : left.name > right.name ? 1 : 0);
    for (const child of children) {
      const source = join(sourceRoot, child.name);
      if (contentType === 'character' && child.name.toLowerCase() === 'template') continue;
      const manifest = await readManifest(source, contentType);
      const archivePath = join(packageRoot, `${manifest.id}-${manifest.version}.zip`);
      const outputKey = basename(archivePath).toLowerCase();
      if (outputNames.has(outputKey)) throw new Error(`duplicate registry archive output: ${basename(archivePath)}`);
      outputNames.add(outputKey);
      const archive = await buildArchive(source, contentType, manifest);
      const entry = registryEntry(manifest, contentType, archive, sourceUrl, packageBaseUrl);
      const artifact = verifyRegistryArtifact(entry, archive);
      if (!artifact.valid) throw new Error(`generated archive ${child.name} is invalid: ${artifact.error}`);
      await writeSafeFile(archivePath, archive, 'registry archive output');
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
  const versionRoot = join(registryRoot, 'v1');
  await ensureSafeDirectory(versionRoot, 'registry index output');
  await writeSafeFile(join(versionRoot, 'index.json'), `${JSON.stringify(index, null, 2)}\n`, 'registry index output');
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
