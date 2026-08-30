// pattern: Functional Core

//! Pure registry archive validation and trust normalization.
//!
//! Network and filesystem orchestration intentionally lives in
//! `commands::registry`; this module only inspects bytes supplied by the shell.

use crate::characters::manifest::{
    is_supported_package_file, validate_package_path, CharacterTemplateManifest,
};
use crate::characters::{MAX_PACKAGE_FILE_COUNT, MAX_PACKAGE_UNCOMPRESSED_BYTES};
use crate::registry::manifest::{
    RegistryEntry, RegistryIndex, OFFICIAL_REGISTRY_IDENTITY, OFFICIAL_REGISTRY_URL,
};
use semver::{Version, VersionReq};
use sha2::{Digest, Sha256};
use std::io::{Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use thiserror::Error;
use zip::ZipArchive;

/// Maximum archive size accepted by a registry download before extraction.
pub const MAX_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum InstallTrust {
    Official,
    Community,
}

/// A validated character archive. The bytes are retained for the imperative
/// shell to pass directly to `CharacterCatalog::install_zip` after validation.
#[derive(Clone, Debug)]
pub struct VerifiedCharacterPackage {
    pub manifest: CharacterTemplateManifest,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RegistryClientError {
    #[error("archive checksum mismatch: expected {expected}, got {actual}")]
    ChecksumMismatch { expected: String, actual: String },
    #[error("archive size mismatch: expected {expected} bytes, got {actual}")]
    SizeMismatch { expected: u64, actual: u64 },
    #[error("archive exceeds download size limit of {MAX_ARCHIVE_BYTES} bytes")]
    ArchiveTooLarge,
    #[error("invalid character archive: {0}")]
    Archive(String),
    #[error("unsafe archive content: {0}")]
    UnsafeContent(String),
    #[error("incompatible character package: {0}")]
    Incompatible(String),
    #[error("invalid character manifest: {0}")]
    Manifest(String),
    #[error("registry entry is not a character package")]
    WrongContentType,
    #[error("registry metadata does not match package manifest: {0}")]
    MetadataMismatch(String),
    #[error("invalid registry index: {0}")]
    InvalidIndex(String),
}

/// Normalize trust at the source boundary. A URL install (`from_registry ==
/// false`) is never official, even when package or URL metadata claims it is.
pub fn install_trust(source_url: &str, from_registry: bool) -> InstallTrust {
    if from_registry && source_url == OFFICIAL_REGISTRY_URL {
        InstallTrust::Official
    } else {
        InstallTrust::Community
    }
}

/// Parse and normalize an index at its source boundary. Only the canonical
/// project endpoint may retain the official label; custom registries cannot
/// self-assert trust through JSON metadata.
pub fn normalize_registry_index(
    json: &str,
    source_url: &str,
) -> Result<RegistryIndex, RegistryClientError> {
    let mut index: RegistryIndex = serde_json::from_str(json)
        .map_err(|error| RegistryClientError::InvalidIndex(error.to_string()))?;
    let official = source_url == OFFICIAL_REGISTRY_URL;
    for entry in &mut index.entries {
        if official && entry.trust == "official" {
            entry.registry_identity = Some(OFFICIAL_REGISTRY_IDENTITY.to_string());
        } else {
            entry.trust = if entry.trust == "unverified" {
                "unverified".to_string()
            } else {
                "community".to_string()
            };
            entry.registry_identity = None;
            entry.trust_source = source_url.to_string();
        }
    }
    index
        .validate()
        .map_err(|error| RegistryClientError::InvalidIndex(error.to_string()))?;
    Ok(index)
}

/// Validate a registry character archive before any extraction takes place.
/// `expected` contains `(archive_size, sha256)` from the trusted index. URL
/// installs may pass `None`, in which case the actual archive size/checksum are
/// accepted but still computed and bounded.
pub fn verify_character_archive(
    bytes: &[u8],
    expected: Option<(u64, String)>,
    engine_version: &Version,
) -> Result<VerifiedCharacterPackage, RegistryClientError> {
    let actual_size = bytes.len() as u64;
    if let Some((expected_size, expected_checksum)) = expected.as_ref() {
        if *expected_size != actual_size {
            return Err(RegistryClientError::SizeMismatch {
                expected: *expected_size,
                actual: actual_size,
            });
        }
        let actual_checksum = sha256_hex(bytes);
        if !expected_checksum.eq_ignore_ascii_case(&actual_checksum) {
            return Err(RegistryClientError::ChecksumMismatch {
                expected: expected_checksum.clone(),
                actual: actual_checksum,
            });
        }
    }
    if actual_size > MAX_ARCHIVE_BYTES {
        return Err(RegistryClientError::ArchiveTooLarge);
    }

    let mut archive = ZipArchive::new(Cursor::new(bytes))
        .map_err(|error| RegistryClientError::Archive(error.to_string()))?;
    let mut total_uncompressed = 0_u64;
    let mut manifest_json = None;
    let mut file_count = 0_usize;

    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|error| RegistryClientError::Archive(error.to_string()))?;
        let name = file.name().to_string();
        let path = normalized_archive_path(&name, file.is_dir());
        validate_package_path(&path)
            .map_err(|error| RegistryClientError::UnsafeContent(error.to_string()))?;
        if !file.is_dir() {
            file_count += 1;
            if file_count > MAX_PACKAGE_FILE_COUNT {
                return Err(RegistryClientError::UnsafeContent(
                    "package file count exceeds limit".to_string(),
                ));
            }
            total_uncompressed = total_uncompressed.checked_add(file.size()).ok_or_else(|| {
                RegistryClientError::UnsafeContent(
                    "package uncompressed size exceeds limit".to_string(),
                )
            })?;
            if total_uncompressed > MAX_PACKAGE_UNCOMPRESSED_BYTES {
                return Err(RegistryClientError::UnsafeContent(
                    "package uncompressed size exceeds limit".to_string(),
                ));
            }
            if !is_supported_package_file(&path) {
                return Err(RegistryClientError::UnsafeContent(
                    path.to_string_lossy().into_owned(),
                ));
            }
            if path == Path::new("character.json") {
                let mut raw = String::new();
                file.read_to_string(&mut raw)
                    .map_err(|error| RegistryClientError::Archive(error.to_string()))?;
                manifest_json = Some(raw);
            }
        }
    }

    let raw = manifest_json.ok_or(RegistryClientError::Manifest(
        "missing character.json".to_string(),
    ))?;
    let manifest = CharacterTemplateManifest::from_json(&raw)
        .map_err(|error| RegistryClientError::Manifest(error.to_string()))?;
    manifest
        .validate_for_engine(engine_version)
        .map_err(|error| match error {
            crate::characters::manifest::ManifestError::IncompatibleEngine { .. } => {
                RegistryClientError::Incompatible(error.to_string())
            }
            other => RegistryClientError::Manifest(other.to_string()),
        })?;

    Ok(VerifiedCharacterPackage {
        manifest,
        bytes: bytes.to_vec(),
    })
}

/// Validate an archive against a registry entry, including the declarative
/// package identity and version metadata.
pub fn verify_registry_entry_archive(
    bytes: &[u8],
    entry: &RegistryEntry,
    engine_version: &Version,
) -> Result<VerifiedCharacterPackage, RegistryClientError> {
    if entry.content_type != "character" {
        return Err(RegistryClientError::WrongContentType);
    }
    let entry_requirement = VersionReq::parse(&entry.engine_version)
        .map_err(|error| RegistryClientError::Incompatible(error.to_string()))?;
    if !entry_requirement.matches(engine_version) {
        return Err(RegistryClientError::Incompatible(format!(
            "registry entry requires engine `{}`, current engine is `{engine_version}`",
            entry.engine_version
        )));
    }
    let package = verify_character_archive(
        bytes,
        Some((entry.archive_size, entry.sha256.clone())),
        engine_version,
    )?;
    if package.manifest.id != entry.id || package.manifest.version != entry.version {
        return Err(RegistryClientError::MetadataMismatch(format!(
            "entry {}@{} does not match manifest {}@{}",
            entry.id, entry.version, package.manifest.id, package.manifest.version
        )));
    }
    if package.manifest.engine_version != entry.engine_version {
        return Err(RegistryClientError::MetadataMismatch(format!(
            "entry engine range `{}` does not match manifest engine range `{}`",
            entry.engine_version, package.manifest.engine_version
        )));
    }
    Ok(package)
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

fn normalized_archive_path(name: &str, is_directory: bool) -> PathBuf {
    if is_directory {
        PathBuf::from(name.trim_end_matches('/'))
    } else {
        PathBuf::from(name)
    }
}

// Keep this generic helper private so archive readers cannot accidentally be
// reused after a mutable borrow of `ZipArchive` has ended.
#[allow(dead_code)]
fn _archive_position<R: Read + Seek>(_archive: &ZipArchive<R>) {}
