// pattern: Imperative Shell

//! Registry HTTP and filesystem orchestration.
//!
//! All archive decisions are delegated to `registry::client` and the existing
//! `CharacterCatalog`; this module owns only network, temporary files, and
//! Tauri path resolution.

use crate::characters::catalog::{CatalogEntry, CharacterCatalog};
use crate::characters::manifest::CharacterTemplateManifest;
use crate::error::KokoroError;
use crate::registry::client::{
    install_trust, normalize_registry_index, verify_character_archive,
    verify_registry_entry_archive, InstallTrust, RegistryClientError, MAX_ARCHIVE_BYTES,
};
use crate::registry::manifest::{RegistryIndex, OFFICIAL_REGISTRY_URL};
use semver::Version;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use tauri::{command, AppHandle, Manager};
use uuid::Uuid;

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

fn engine_version() -> Version {
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

fn normalize_registry_url(url: Option<String>) -> Result<String, KokoroError> {
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

async fn fetch_bytes(url: &str) -> Result<Vec<u8>, KokoroError> {
    let parsed = reqwest::Url::parse(url)
        .map_err(|error| KokoroError::Validation(format!("invalid download URL: {error}")))?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return Err(KokoroError::Validation(
            "registry downloads must use HTTPS without credentials".to_string(),
        ));
    }
    let response = reqwest::Client::new()
        .get(parsed)
        .send()
        .await
        .map_err(|error| {
            KokoroError::ExternalService(format!("failed to download registry content: {error}"))
        })?
        .error_for_status()
        .map_err(|error| {
            KokoroError::ExternalService(format!("registry download failed: {error}"))
        })?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_ARCHIVE_BYTES)
    {
        return Err(KokoroError::Validation(format!(
            "archive exceeds download size limit of {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    let bytes = response.bytes().await.map_err(|error| {
        KokoroError::ExternalService(format!("failed to read registry content: {error}"))
    })?;
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(KokoroError::Validation(format!(
            "archive exceeds download size limit of {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    Ok(bytes.to_vec())
}

async fn fetch_index(source_url: &str) -> Result<RegistryIndex, KokoroError> {
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
    let response = reqwest::Client::new()
        .get(parsed)
        .send()
        .await
        .map_err(|error| {
            KokoroError::ExternalService(format!("failed to fetch registry index: {error}"))
        })?
        .error_for_status()
        .map_err(|error| {
            KokoroError::ExternalService(format!("registry index request failed: {error}"))
        })?;
    response.text().await.map_err(|error| {
        KokoroError::ExternalService(format!("failed to read registry index: {error}"))
    })
}

fn persist_download_temp(bytes: &[u8]) -> Result<PathBuf, KokoroError> {
    if bytes.len() as u64 > MAX_ARCHIVE_BYTES {
        return Err(KokoroError::Validation(format!(
            "archive exceeds download size limit of {MAX_ARCHIVE_BYTES} bytes"
        )));
    }
    let target = std::env::temp_dir().join(format!("kokoro-registry-{}.zip", Uuid::new_v4()));
    fs::write(&target, bytes)
        .map_err(|error| KokoroError::Io(format!("failed to stage registry download: {error}")))?;
    Ok(target)
}

fn client_error(error: RegistryClientError) -> KokoroError {
    KokoroError::Validation(error.to_string())
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
    install_verified_package(&app, verified, install_trust(&source_url, true))
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
    fs::write(
        &temporary,
        serde_json::to_vec(&serde_json::json!({ "character_id": id }))?,
    )
    .map_err(|error| {
        KokoroError::Io(format!(
            "failed to stage active character fallback: {error}"
        ))
    })?;
    fs::rename(&temporary, &target).map_err(|error| {
        KokoroError::Io(format!(
            "failed to apply active character fallback: {error}"
        ))
    })
}

fn choose_fallback(catalog: &CharacterCatalog, removed_id: &str) -> Option<String> {
    catalog
        .discover()
        .ok()?
        .into_iter()
        .find(|entry| entry.manifest.id != removed_id)
        .map(|entry| entry.manifest.id)
        .or_else(|| Some("kokoro".to_string()).filter(|id| id != removed_id))
}

#[command]
pub async fn remove_character_package(
    app: AppHandle,
    character_id: String,
    version: String,
) -> Result<CharacterPackageRemoval, KokoroError> {
    let app_data = app_data_dir(&app)?;
    let catalog = CharacterCatalog::new(app_data.join("characters"), engine_version());
    let active = read_active_character_id(&app_data);
    let fallback = if active.as_deref() == Some(character_id.as_str()) {
        choose_fallback(&catalog, &character_id)
    } else {
        None
    };
    if let Some(id) = fallback.as_deref() {
        write_active_character_id(&app_data, id)?;
    }
    catalog
        .remove_package(&character_id, &version)
        .map_err(|error| {
            KokoroError::Validation(format!("failed to remove character package: {error}"))
        })?;
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
