// pattern: Imperative Shell

//! Registry HTTP and filesystem orchestration.
//!
//! All archive decisions are delegated to `registry::client` and the existing
//! `CharacterCatalog`; this module owns only network, temporary files, and
//! Tauri path resolution.

use crate::characters::catalog::{CatalogEntry, CharacterCatalog};
use crate::characters::manifest::CharacterTemplateManifest;
use crate::commands::characters::activate_character_for_package_removal;
use crate::error::KokoroError;
use crate::registry::client::{
    normalize_registry_index, verify_character_archive, verify_registry_entry_archive,
    InstallTrust, RegistryClientError, MAX_ARCHIVE_BYTES,
};
use crate::registry::manifest::{
    RegistryEntry, RegistryIndex, OFFICIAL_PACKAGE_BASE_URL, OFFICIAL_REGISTRY_IDENTITY,
    OFFICIAL_REGISTRY_URL,
};
use futures::StreamExt;
use semver::Version;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tauri::{command, AppHandle, Manager, State};
use uuid::Uuid;

const MAX_REGISTRY_INDEX_BYTES: u64 = 4 * 1024 * 1024;
pub(crate) const REGISTRY_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);
pub(crate) const REGISTRY_REQUEST_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(60);

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledCharacterPackage {
    pub id: String,
    pub version: String,
    pub name: String,
    pub trust: String,
    pub package_dir: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct CharacterPackageRemoval {
    pub id: String,
    pub version: String,
    pub active_fallback: Option<String>,
}

pub(crate) fn engine_version() -> Version {
    Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 3, 1))
}

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, KokoroError> {
    app.path()
        .app_data_dir()
        .map_err(|error| KokoroError::Io(format!("failed to resolve app data directory: {error}")))
}

fn catalog_for_app(app: &AppHandle) -> Result<CharacterCatalog, KokoroError> {
    Ok(CharacterCatalog::new(
        app_data_dir(app)?.join("characters"),
        engine_version(),
    ))
}

pub(crate) fn normalize_registry_url(url: Option<String>) -> Result<String, KokoroError> {
    let value = url.unwrap_or_else(|| OFFICIAL_REGISTRY_URL.to_string());
    let parsed = reqwest::Url::parse(&value)
        .map_err(|error| KokoroError::Validation(format!("invalid registry URL: {error}")))?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return Err(KokoroError::Validation(
            "registry URL must use HTTPS without credentials".to_string(),
        ));
    }
    Ok(value)
}

/// Registry and package downloads never follow redirects.  The registry
/// identity is derived from the requested endpoint, so silently following a
/// redirect could otherwise turn an official URL into an untrusted origin.
fn registry_http_client() -> Result<reqwest::Client, KokoroError> {
    reqwest::Client::builder()
        .connect_timeout(REGISTRY_CONNECT_TIMEOUT)
        .timeout(REGISTRY_REQUEST_TIMEOUT)
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .map_err(|error| {
            KokoroError::ExternalService(format!("failed to configure registry client: {error}"))
        })
}

pub(crate) async fn fetch_bytes(url: &str) -> Result<Vec<u8>, KokoroError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| KokoroError::Validation(format!("invalid download URL: {error}")))?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return Err(KokoroError::Validation(
            "registry downloads must use HTTPS without credentials".to_string(),
        ));
    }
    let response = registry_http_client()?
        .get(parsed)
        .send()
        .await
        .map_err(|error| {
            KokoroError::ExternalService(format!("failed to download registry content: {error}"))
        })?;
    require_success_status(response.status(), "registry download")?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE_BYTES)
    {
        return Err(KokoroError::Validation(format!(
            "archive exceeds download size limit of {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    read_response_bytes(response, MAX_ARCHIVE_BYTES, "archive").await
}

pub(crate) async fn fetch_index(source_url: &str) -> Result<RegistryIndex, KokoroError> {
    let json = fetch_text(source_url).await?;
    normalize_registry_index(&json, source_url).map_err(client_error)
}

async fn fetch_text(url: &str) -> Result<String, KokoroError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| KokoroError::Validation(format!("invalid registry URL: {error}")))?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return Err(KokoroError::Validation(
            "registry URL must use HTTPS without credentials".to_string(),
        ));
    }
    let response = registry_http_client()?
        .get(parsed)
        .send()
        .await
        .map_err(|error| {
            KokoroError::ExternalService(format!("failed to fetch registry index: {error}"))
        })?;
    require_success_status(response.status(), "registry index request")?;
    let bytes = read_response_bytes(response, MAX_REGISTRY_INDEX_BYTES, "registry index").await?;
    String::from_utf8(bytes).map_err(|error| {
        KokoroError::Validation(format!("registry index is not valid UTF-8: {error}"))
    })
}

pub(crate) fn require_success_status(
    status: reqwest::StatusCode,
    label: &str,
) -> Result<(), KokoroError> {
    if status.is_success() {
        Ok(())
    } else {
        Err(KokoroError::ExternalService(format!(
            "{label} returned unexpected HTTP status {status}"
        )))
    }
}

async fn read_response_bytes(
    response: reqwest::Response,
    limit: u64,
    label: &str,
) -> Result<Vec<u8>, KokoroError> {
    if response.content_length().is_some_and(|size| size > limit) {
        return Err(KokoroError::Validation(format!(
            "{label} exceeds download size limit of {limit} bytes"
        )));
    }
    let mut stream = response.bytes_stream();
    let mut bytes = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|error| {
            KokoroError::ExternalService(format!("failed to read {label}: {error}"))
        })?;
        if append_limited_chunk(&mut bytes, &chunk, limit).is_err() {
            return Err(KokoroError::Validation(format!(
                "{label} exceeds download size limit of {limit} bytes"
            )));
        }
    }
    Ok(bytes)
}

pub(crate) fn append_limited_chunk(
    buffer: &mut Vec<u8>,
    chunk: &[u8],
    limit: u64,
) -> Result<(), ()> {
    let next_size = u64::try_from(buffer.len())
        .ok()
        .and_then(|size| size.checked_add(chunk.len() as u64))
        .ok_or(())?;
    if next_size > limit {
        return Err(());
    }
    buffer.extend_from_slice(chunk);
    Ok(())
}

fn persist_download_temp(bytes: &[u8]) -> Result<PathBuf, KokoroError> {
    let target = std::env::temp_dir().join(format!("kokoro-registry-{}.zip", Uuid::new_v4()));
    persist_download_temp_at(bytes, &target)
}

pub(crate) fn persist_download_temp_at(
    bytes: &[u8],
    target: &Path,
) -> Result<PathBuf, KokoroError> {
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(KokoroError::Validation(format!(
            "archive exceeds download size limit of {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|error| {
            let message = if error.kind() == std::io::ErrorKind::AlreadyExists {
                format!(
                    "temporary download path already exists: {}",
                    target.display()
                )
            } else {
                format!("failed to stage registry download: {error}")
            };
            KokoroError::Io(message)
        })?;
    use std::io::Write;
    file.write_all(bytes)
        .map_err(|error| KokoroError::Io(format!("failed to stage registry download: {error}")))?;
    file.sync_all().map_err(|error| {
        KokoroError::Io(format!("failed to finalize registry download: {error}"))
    })?;
    Ok(target.to_path_buf())
}

pub(crate) fn revalidate_staged_character_bytes(
    original: &crate::registry::client::VerifiedCharacterPackage,
    staged: &[u8],
    engine: &Version,
) -> Result<crate::registry::client::VerifiedCharacterPackage, KokoroError> {
    let expected = (
        original.bytes.len() as u64,
        crate::registry::client::sha256_hex(&original.bytes),
    );
    let revalidated =
        verify_character_archive(staged, Some(expected), engine).map_err(client_error)?;
    if revalidated.manifest != original.manifest {
        return Err(KokoroError::Validation(
            "staged registry archive changed before installation".to_string(),
        ));
    }
    Ok(revalidated)
}

fn client_error(error: RegistryClientError) -> KokoroError {
    KokoroError::Validation(error.to_string())
}

pub(crate) fn trust_for_registry_entry(source_url: &str, entry: &RegistryEntry) -> InstallTrust {
    if source_url == OFFICIAL_REGISTRY_URL
        && entry.validate().is_ok()
        && entry.trust == "official"
        && entry.trust_source == OFFICIAL_REGISTRY_URL
        && entry.registry_identity.as_deref() == Some(OFFICIAL_REGISTRY_IDENTITY)
        && official_package_url_matches_entry(entry)
    {
        InstallTrust::Official
    } else {
        InstallTrust::Community
    }
}

fn official_package_url_matches_entry(entry: &RegistryEntry) -> bool {
    let Ok(base) = reqwest::Url::parse(OFFICIAL_PACKAGE_BASE_URL) else {
        return false;
    };
    let Ok(download) = reqwest::Url::parse(&entry.download_url) else {
        return false;
    };
    download.scheme() == base.scheme()
        && download.host_str() == base.host_str()
        && download.port() == base.port()
        && download.username().is_empty()
        && download.password().is_none()
        && download.query().is_none()
        && download.fragment().is_none()
        && download.path()
            == format!(
                "{}/{}-{}.zip",
                base.path().trim_end_matches('/'),
                entry.id,
                entry.version
            )
}

fn installed_result(entry: CatalogEntry, trust: InstallTrust) -> InstalledCharacterPackage {
    InstalledCharacterPackage {
        id: entry.manifest.id,
        version: entry.manifest.version,
        name: entry.manifest.name,
        trust: match trust {
            InstallTrust::Official => "official".to_string(),
            InstallTrust::Community => "community".to_string(),
        },
        package_dir: entry.package_dir.to_string_lossy().into_owned(),
    }
}

/// Removing a package must not silently switch the user's active instance to
/// another character.  The activation coordinator re-applies that same
/// instance while package presentation assets are unavailable, allowing its
/// built-in presentation fallback to take over.
pub(crate) fn removal_activation_target(
    active_id: Option<&str>,
    uses_removed_package: bool,
) -> Option<String> {
    if uses_removed_package {
        active_id.map(ToOwned::to_owned)
    } else {
        None
    }
}

#[command]
pub async fn list_registry_entries(
    registry_url: Option<String>,
) -> Result<RegistryIndex, KokoroError> {
    let source_url = normalize_registry_url(registry_url)?;
    fetch_index(&source_url).await
}

#[command]
pub async fn install_character_from_registry(
    app: AppHandle,
    character_id: String,
    version: String,
    registry_url: Option<String>,
) -> Result<InstalledCharacterPackage, KokoroError> {
    let source_url = normalize_registry_url(registry_url)?;
    let index = fetch_index(&source_url).await?;
    let entry = index
        .entries
        .iter()
        .find(|entry| {
            entry.content_type == "character"
                && entry.id == character_id
                && entry.version == version
        })
        .cloned()
        .ok_or_else(|| {
            KokoroError::NotFound(format!(
                "character package '{character_id}@{version}' not found"
            ))
        })?;
    let bytes = fetch_bytes(&entry.download_url).await?;
    let verified =
        verify_registry_entry_archive(&bytes, &entry, &engine_version()).map_err(client_error)?;
    install_verified_package(
        &app,
        verified,
        trust_for_registry_entry(&source_url, &entry),
    )
}

#[command]
pub async fn install_character_from_url(
    app: AppHandle,
    url: String,
) -> Result<InstalledCharacterPackage, KokoroError> {
    let bytes = fetch_bytes(&url).await?;
    let verified =
        verify_character_archive(&bytes, None, &engine_version()).map_err(client_error)?;
    install_verified_package(&app, verified, InstallTrust::Community)
}

fn install_verified_package(
    app: &AppHandle,
    verified: crate::registry::client::VerifiedCharacterPackage,
    trust: InstallTrust,
) -> Result<InstalledCharacterPackage, KokoroError> {
    let temporary = persist_download_temp(&verified.bytes)?;
    let result = (|| {
        let bytes = fs::read(&temporary)
            .map_err(|error| KokoroError::Io(format!("failed to read staged archive: {error}")))?;
        let _revalidated = revalidate_staged_character_bytes(&verified, &bytes, &engine_version())?;
        let catalog = catalog_for_app(app)?;
        let entry = catalog.install_zip(Cursor::new(bytes)).map_err(|error| {
            KokoroError::Validation(format!("failed to install character package: {error}"))
        })?;
        Ok(installed_result(entry, trust))
    })();
    let _ = fs::remove_file(&temporary);
    result
}

fn read_active_character_id(app_data: &Path) -> Option<String> {
    let path = app_data.join("active_character_id.json");
    let value: serde_json::Value = serde_json::from_slice(&fs::read(path).ok()?).ok()?;
    value.get("character_id")?.as_str().map(ToOwned::to_owned)
}

fn write_active_character_id(app_data: &Path, id: &str) -> Result<(), KokoroError> {
    let target = app_data.join("active_character_id.json");
    let temporary = app_data.join(format!(".active-character-{}.tmp", Uuid::new_v4()));
    let backup = app_data.join(format!(".active-character-{}.backup", Uuid::new_v4()));
    fs::write(
        &temporary,
        serde_json::to_vec(&serde_json::json!({ "character_id": id }))?,
    )
    .map_err(|error| {
        KokoroError::Io(format!(
            "failed to stage active character fallback: {error}"
        ))
    })?;
    if !target.exists() {
        return fs::rename(&temporary, &target).map_err(|error| {
            let _ = fs::remove_file(&temporary);
            KokoroError::Io(format!(
                "failed to apply active character fallback: {error}"
            ))
        });
    }
    fs::rename(&target, &backup).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        KokoroError::Io(format!(
            "failed to stage active character fallback: {error}"
        ))
    })?;
    if let Err(error) = fs::rename(&temporary, &target) {
        let _ = fs::rename(&backup, &target);
        let _ = fs::remove_file(&temporary);
        return Err(KokoroError::Io(format!(
            "failed to apply active character fallback: {error}"
        )));
    }
    fs::remove_file(&backup).map_err(|error| {
        KokoroError::Io(format!(
            "failed to finalize active character fallback: {error}"
        ))
    })
}

async fn active_character_uses_package(
    pool: &SqlitePool,
    active_id: Option<&str>,
    package_id: &str,
    package_version: &str,
) -> Result<bool, KokoroError> {
    let Some(active_id) = active_id else {
        return Ok(false);
    };
    let origin = sqlx::query_as::<_, (Option<String>, Option<String>)>(
        "SELECT template_id, template_version FROM characters WHERE id = ?",
    )
    .bind(active_id)
    .fetch_optional(pool)
    .await?;
    Ok(match origin {
        Some((template_id, template_version)) => {
            template_id.as_deref() == Some(package_id)
                && template_version.as_deref() == Some(package_version)
        }
        // Legacy installations may have no character row for the active
        // package id. Preserve that conservative compatibility behavior.
        None => active_id == package_id,
    })
}

#[command]
pub async fn remove_character_package(
    app: AppHandle,
    character_id: String,
    version: String,
    coordinator: State<'_, crate::characters::activation::ActivationCoordinator>,
    orchestrator: State<'_, crate::ai::context::AIOrchestrator>,
) -> Result<CharacterPackageRemoval, KokoroError> {
    let app_data = app_data_dir(&app)?;
    let catalog = CharacterCatalog::new(app_data.join("characters"), engine_version());
    let active = read_active_character_id(&app_data);
    let is_active =
        active_character_uses_package(&orchestrator.db, active.as_deref(), &character_id, &version)
            .await?;
    let staged = catalog
        .stage_package_removal(&character_id, &version)
        .map_err(|error| {
            KokoroError::Validation(format!(
                "failed to stage character package removal: {error}"
            ))
        })?;
    let is_active = staged.is_some() && is_active;
    let fallback = removal_activation_target(active.as_deref(), is_active);
    let previous_active = active.clone();
    let Some(staged) = staged else {
        return Ok(CharacterPackageRemoval {
            id: character_id,
            version,
            active_fallback: None,
        });
    };
    if is_active {
        let fallback_id = match fallback.as_deref() {
            Some(id) => id,
            None => {
                let _ = staged.rollback();
                return Err(KokoroError::Validation(
                    "cannot remove active character package without a fallback instance"
                        .to_string(),
                ));
            }
        };
        if let Err(error) = activate_character_for_package_removal(
            &coordinator,
            &orchestrator,
            &app_data,
            fallback_id,
        )
        .await
        {
            let rollback_error = staged.rollback().err();
            let active_restore_error = previous_active
                .as_deref()
                .and_then(|previous| write_active_character_id(&app_data, previous).err());
            if let Some(rollback_error) = rollback_error {
                return Err(KokoroError::Validation(format!(
                    "character activation failed: {error}; failed to restore package: {rollback_error}"
                )));
            }
            if let Some(active_restore_error) = active_restore_error {
                return Err(KokoroError::Validation(format!(
                    "character activation failed: {error}; failed to restore active character: {active_restore_error}"
                )));
            }
            return Err(error);
        }
    }
    if let Err(error) = staged.finalize() {
        let active_restore_error = if let Some(previous) = previous_active.as_deref() {
            if let Err(active_error) = write_active_character_id(&app_data, previous) {
                Some(active_error)
            } else if is_active {
                activate_character_for_package_removal(
                    &coordinator,
                    &orchestrator,
                    &app_data,
                    previous,
                )
                .await
                .err()
            } else {
                None
            }
        } else {
            None
        };
        if let Some(active_restore_error) = active_restore_error {
            return Err(KokoroError::Validation(format!(
                "failed to remove character package: {error}; failed to restore active character: {active_restore_error}"
            )));
        }
        return Err(KokoroError::Validation(format!(
            "failed to remove character package: {error}"
        )));
    }
    Ok(CharacterPackageRemoval {
        id: character_id,
        version,
        active_fallback: fallback,
    })
}

/// Exposed for backup restore orchestration: local exact versions are preferred;
/// callers use built-in presentation when this returns `None`.
pub fn resolve_local_character_version(
    app_data: &Path,
    id: &str,
    version: &str,
) -> Option<CharacterTemplateManifest> {
    CharacterCatalog::new(app_data.join("characters"), engine_version())
        .find_exact(id, version)
        .map(|entry| entry.manifest)
}
