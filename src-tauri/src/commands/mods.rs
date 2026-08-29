// pattern: Imperative Shell

use crate::error::KokoroError;
use crate::mods::{ModManager, ModManifest, ModThemeJson};
use crate::registry::manifest::RegistryEntry;
use semver::Version;
use serde_json::Value as JsonValue;
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Cursor, Read, Seek};
use std::path::{Path, PathBuf};
use tauri::{command, AppHandle, State};
use tokio::sync::Mutex;
use uuid::Uuid;

/// Origin controls trust messaging. URL MODs always require a separate
/// acknowledgement because checksum/transport validation cannot establish
/// that executable code is safe.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ModInstallSource {
    Local,
    Registry,
    Url,
}

#[derive(Debug)]
pub struct ModInstallResult {
    pub manifest: ModManifest,
    pub source: ModInstallSource,
}

const MAX_MOD_ARCHIVE_BYTES: u64 = 64 * 1024 * 1024;

/// Validate mod ID format: must be non-empty and contain only alphanumeric, underscore, or dash
fn is_valid_mod_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_alphanumeric() || c == '_' || c == '-')
}

/// Check if a file extension is allowed for MOD extraction
fn is_allowed_mod_file(ext: &str) -> bool {
    const ALLOWED_EXTENSIONS: &[&str] = &[
        "html", "js", "css", "json", "png", "jpg", "jpeg", "webp", "svg", "gif", "woff", "woff2",
        "ttf", "otf", "txt", "md",
    ];
    ALLOWED_EXTENSIONS.contains(&ext.to_lowercase().as_str())
}

pub fn untrusted_mod_url_warning(url: &str) -> String {
    format!(
        "This URL install contains untrusted executable MOD code ({url}). Review requested permissions and continue only if you trust the source."
    )
}

fn permission_review(
    manifest: &ModManifest,
    source: ModInstallSource,
    confirmed: bool,
) -> Result<(), KokoroError> {
    let requires_confirmation =
        source == ModInstallSource::Url || manifest.permission_review_required();
    if requires_confirmation && !confirmed {
        let reason = if source == ModInstallSource::Url {
            untrusted_mod_url_warning("the supplied URL")
        } else {
            "This MOD requests permissions that require explicit review before installation."
                .to_string()
        };
        return Err(KokoroError::Unauthorized(reason));
    }
    Ok(())
}

fn read_manifest<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<ModManifest, KokoroError> {
    let mut content = String::new();
    let mut file = archive
        .by_name("mod.json")
        .map_err(|_| KokoroError::Validation("mod.json not found in archive root".to_string()))?;
    file.read_to_string(&mut content)
        .map_err(KokoroError::from)?;
    let manifest: ModManifest = serde_json::from_str(&content)
        .map_err(|error| KokoroError::Validation(format!("Invalid mod.json: {error}")))?;
    Ok(manifest)
}

fn extract_to_staging(
    bytes: &[u8],
    mods_dir: &Path,
    engine_version: &Version,
) -> Result<(ModManifest, PathBuf), KokoroError> {
    let file = Cursor::new(bytes);
    let mut archive = zip::ZipArchive::new(file).map_err(KokoroError::from)?;
    let manifest = read_manifest(&mut archive)?;
    manifest
        .validate_for_engine(engine_version)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;

    let staging_dir = mods_dir.join(format!(".staging-{}", Uuid::new_v4()));
    fs::create_dir_all(&staging_dir).map_err(KokoroError::from)?;
    const MAX_FILE_SIZE: u64 = 10 * 1024 * 1024;
    const MAX_TOTAL_SIZE: u64 = 50 * 1024 * 1024;
    let mut total_size = 0_u64;
    let extraction = (|| -> Result<(), KokoroError> {
        for index in 0..archive.len() {
            let mut file = archive
                .by_index(index)
                .map_err(|error| KokoroError::Validation(error.to_string()))?;
            let relative = file.enclosed_name().ok_or_else(|| {
                KokoroError::Validation(format!("unsafe archive path `{}`", file.name()))
            })?;
            if relative.is_absolute()
                || relative
                    .components()
                    .any(|component| matches!(component, std::path::Component::ParentDir))
            {
                return Err(KokoroError::Validation(format!(
                    "unsafe archive path `{}`",
                    file.name()
                )));
            }
            let outpath = staging_dir.join(relative);
            if file.name().ends_with('/') {
                fs::create_dir_all(&outpath).map_err(KokoroError::from)?;
                continue;
            }
            let ext = outpath
                .extension()
                .and_then(|extension| extension.to_str())
                .unwrap_or_default();
            if !is_allowed_mod_file(ext) {
                return Err(KokoroError::Validation(format!(
                    "unsupported MOD file `{}`",
                    file.name()
                )));
            }
            if file.size() > MAX_FILE_SIZE {
                return Err(KokoroError::Validation(format!(
                    "MOD file `{}` exceeds the 10MB limit",
                    file.name()
                )));
            }
            total_size = total_size.saturating_add(file.size());
            if total_size > MAX_TOTAL_SIZE {
                return Err(KokoroError::Validation(
                    "MOD package exceeds the 50MB limit".to_string(),
                ));
            }
            if let Some(parent) = outpath.parent() {
                fs::create_dir_all(parent).map_err(KokoroError::from)?;
            }
            let mut output = fs::File::create(&outpath).map_err(KokoroError::from)?;
            io::copy(&mut file, &mut output).map_err(KokoroError::from)?;
        }
        Ok(())
    })();
    if let Err(error) = extraction {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    Ok((manifest, staging_dir))
}

fn inspect_mod_archive(bytes: &[u8], engine_version: &Version) -> Result<ModManifest, KokoroError> {
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(KokoroError::from)?;
    let manifest = read_manifest(&mut archive)?;
    manifest
        .validate_for_engine(engine_version)
        .map_err(|error| KokoroError::Validation(error.to_string()))?;
    Ok(manifest)
}

/// Validate and atomically replace an installed MOD. The old directory is
/// untouched until extraction and permission review have both succeeded.
pub fn install_mod_archive(
    bytes: &[u8],
    mods_dir: &Path,
    engine_version: &Version,
    permission_confirmed: bool,
    source: ModInstallSource,
) -> Result<ModInstallResult, KokoroError> {
    if bytes.len() as u64 > MAX_MOD_ARCHIVE_BYTES {
        return Err(KokoroError::Validation(format!(
            "MOD archive exceeds the {}MB download limit",
            MAX_MOD_ARCHIVE_BYTES / (1024 * 1024)
        )));
    }
    fs::create_dir_all(mods_dir).map_err(KokoroError::from)?;
    let (manifest, staging_dir) = extract_to_staging(bytes, mods_dir, engine_version)?;
    if let Err(error) = permission_review(&manifest, source, permission_confirmed) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }
    let target = mods_dir.join(&manifest.id);
    let previous = mods_dir.join(format!(".previous-{}", Uuid::new_v4()));
    let had_previous = target.exists();
    if had_previous {
        fs::rename(&target, &previous).map_err(|error| {
            let _ = fs::remove_dir_all(&staging_dir);
            KokoroError::Io(format!("failed to stage previous MOD: {error}"))
        })?;
    }
    if let Err(error) = fs::rename(&staging_dir, &target) {
        if had_previous {
            let _ = fs::rename(&previous, &target);
        }
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(KokoroError::Io(format!(
            "failed to install staged MOD: {error}"
        )));
    }
    if had_previous {
        let _ = fs::remove_dir_all(previous);
    }
    Ok(ModInstallResult { manifest, source })
}

/// Verify registry metadata before handing a MOD archive to the same staged
/// installer used by local and URL sources. Registry metadata is not trusted
/// merely because it arrived over HTTPS: size, checksum, identity, version,
/// and engine compatibility are all checked against the package itself.
pub fn install_registry_mod_archive(
    bytes: &[u8],
    entry: &RegistryEntry,
    mods_dir: &Path,
    engine_version: &Version,
    permission_confirmed: bool,
) -> Result<ModInstallResult, KokoroError> {
    entry
        .validate()
        .map_err(|error| KokoroError::Validation(error.to_string()))?;
    if entry.content_type != "mod" {
        return Err(KokoroError::Validation(
            "registry entry is not a MOD package".to_string(),
        ));
    }
    if entry.archive_size != bytes.len() as u64 {
        return Err(KokoroError::Validation(format!(
            "MOD archive size mismatch: expected {}, got {}",
            entry.archive_size,
            bytes.len()
        )));
    }
    let actual_checksum = format!("{:x}", Sha256::digest(bytes));
    if !entry.sha256.eq_ignore_ascii_case(&actual_checksum) {
        return Err(KokoroError::Validation(format!(
            "MOD archive checksum mismatch: expected {}, got {actual_checksum}",
            entry.sha256
        )));
    }
    let candidate = inspect_mod_archive(bytes, engine_version)?;
    if candidate.id != entry.id || candidate.version != entry.version {
        return Err(KokoroError::Validation(format!(
            "registry MOD metadata does not match manifest {}@{}",
            candidate.id, candidate.version
        )));
    }
    install_mod_archive(
        bytes,
        mods_dir,
        engine_version,
        permission_confirmed,
        ModInstallSource::Registry,
    )
}

pub fn remove_installed_mod(mods_dir: &Path, mod_id: &str) -> Result<(), KokoroError> {
    if !is_valid_mod_id(mod_id) {
        return Err(KokoroError::Validation("invalid MOD id".to_string()));
    }
    let target = mods_dir.join(mod_id);
    if !target.exists() {
        return Err(KokoroError::NotFound(format!("MOD '{mod_id}' not found")));
    }
    fs::remove_dir_all(target).map_err(|error| KokoroError::Io(error.to_string()))
}

#[command]
pub async fn list_mods(
    mod_manager: State<'_, Mutex<ModManager>>,
) -> Result<Vec<ModManifest>, KokoroError> {
    let mut manager = mod_manager.lock().await;
    Ok(manager.scan_mods())
}

#[command]
pub async fn load_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    app_handle: AppHandle,
    mod_id: String,
) -> Result<(), KokoroError> {
    let mut manager = mod_manager.lock().await;
    manager
        .load_mod(&mod_id, &app_handle)
        .await
        .map_err(KokoroError::Mod)
}

#[command]
pub async fn get_mod_theme(
    mod_manager: State<'_, Mutex<ModManager>>,
) -> Result<Option<ModThemeJson>, KokoroError> {
    let manager = mod_manager.lock().await;
    Ok(manager.get_active_theme().cloned())
}

#[command]
pub async fn get_mod_layout(
    mod_manager: State<'_, Mutex<ModManager>>,
) -> Result<Option<JsonValue>, KokoroError> {
    let manager = mod_manager.lock().await;
    Ok(manager.get_active_layout().cloned())
}

#[command]
pub async fn install_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    file_path: String,
) -> Result<ModManifest, KokoroError> {
    let mods_dir = {
        let manager = mod_manager.lock().await;
        manager.mods_path.clone()
    };

    let archive_path = Path::new(&file_path);
    if !archive_path.exists() {
        return Err(KokoroError::NotFound("File does not exist".to_string()));
    }
    let bytes = fs::read(archive_path).map_err(KokoroError::from)?;
    let result = install_mod_archive(
        &bytes,
        &mods_dir,
        &Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 3, 1)),
        true,
        ModInstallSource::Local,
    )?;
    Ok(result.manifest)
}

#[command]
pub async fn update_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    mod_id: String,
    file_path: String,
    permission_confirmed: bool,
) -> Result<ModManifest, KokoroError> {
    let mods_dir = {
        let manager = mod_manager.lock().await;
        manager.mods_path.clone()
    };
    let bytes = fs::read(&file_path).map_err(KokoroError::from)?;
    let engine =
        Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 3, 1));
    let candidate = inspect_mod_archive(&bytes, &engine)?;
    if candidate.id != mod_id {
        return Err(KokoroError::Validation(format!(
            "update package id '{}' does not match requested MOD '{mod_id}'",
            candidate.id
        )));
    }
    let result = install_mod_archive(
        &bytes,
        &mods_dir,
        &engine,
        permission_confirmed,
        ModInstallSource::Registry,
    )?;
    Ok(result.manifest)
}

#[command]
pub async fn remove_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    mod_id: String,
) -> Result<(), KokoroError> {
    let mods_dir = {
        let manager = mod_manager.lock().await;
        manager.mods_path.clone()
    };
    remove_installed_mod(&mods_dir, &mod_id)
}

#[command]
pub async fn install_mod_from_url(
    mod_manager: State<'_, Mutex<ModManager>>,
    url: String,
    confirm_untrusted_code: bool,
) -> Result<ModManifest, KokoroError> {
    let parsed = reqwest::Url::parse(&url)
        .map_err(|error| KokoroError::Validation(format!("invalid MOD URL: {error}")))?;
    if parsed.scheme() != "https" || parsed.username() != "" || parsed.password().is_some() {
        return Err(KokoroError::Validation(
            "MOD URL installs require HTTPS without credentials".to_string(),
        ));
    }
    if !confirm_untrusted_code {
        return Err(KokoroError::Unauthorized(untrusted_mod_url_warning(&url)));
    }
    let response = reqwest::Client::new()
        .get(parsed)
        .send()
        .await
        .map_err(|error| KokoroError::ExternalService(error.to_string()))?
        .error_for_status()
        .map_err(|error| KokoroError::ExternalService(error.to_string()))?;
    if response
        .content_length()
        .is_some_and(|size| size > MAX_MOD_ARCHIVE_BYTES)
    {
        return Err(KokoroError::Validation(format!(
            "MOD archive exceeds the {}MB download limit",
            MAX_MOD_ARCHIVE_BYTES / (1024 * 1024)
        )));
    }
    let bytes = response
        .bytes()
        .await
        .map_err(|error| KokoroError::ExternalService(error.to_string()))?;
    if bytes.len() as u64 > MAX_MOD_ARCHIVE_BYTES {
        return Err(KokoroError::Validation(format!(
            "MOD archive exceeds the {}MB download limit",
            MAX_MOD_ARCHIVE_BYTES / (1024 * 1024)
        )));
    }
    let mods_dir = {
        let manager = mod_manager.lock().await;
        manager.mods_path.clone()
    };
    let result = install_mod_archive(
        &bytes,
        &mods_dir,
        &Version::parse(env!("CARGO_PKG_VERSION")).unwrap_or_else(|_| Version::new(0, 3, 1)),
        true,
        ModInstallSource::Url,
    )?;
    Ok(result.manifest)
}

#[command]
pub async fn dispatch_mod_event(
    mod_manager: State<'_, Mutex<ModManager>>,
    event: String,
    payload: JsonValue,
) -> Result<(), KokoroError> {
    let manager = mod_manager.lock().await;
    manager
        .dispatch_event(&event, payload)
        .await
        .map_err(KokoroError::Mod)
}

#[command]
pub async fn unload_mod(
    mod_manager: State<'_, Mutex<ModManager>>,
    app_handle: AppHandle,
) -> Result<(), KokoroError> {
    let mut manager = mod_manager.lock().await;
    manager.unload_mod(&app_handle).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_valid_mod_id_empty() {
        assert!(!is_valid_mod_id(""), "Empty ID should be invalid");
    }

    #[test]
    fn test_is_valid_mod_id_valid_alphanumeric() {
        assert!(is_valid_mod_id("mymod"), "Alphanumeric ID should be valid");
        assert!(
            is_valid_mod_id("MyMod123"),
            "Mixed case alphanumeric should be valid"
        );
    }

    #[test]
    fn test_is_valid_mod_id_valid_with_underscore() {
        assert!(
            is_valid_mod_id("my_mod"),
            "ID with underscore should be valid"
        );
        assert!(
            is_valid_mod_id("_private_mod"),
            "ID starting with underscore should be valid"
        );
    }

    #[test]
    fn test_is_valid_mod_id_valid_with_dash() {
        assert!(is_valid_mod_id("my-mod"), "ID with dash should be valid");
        assert!(
            is_valid_mod_id("my-mod-123"),
            "ID with multiple dashes should be valid"
        );
    }

    #[test]
    fn test_is_valid_mod_id_invalid_special_chars() {
        assert!(!is_valid_mod_id("my.mod"), "ID with dot should be invalid");
        assert!(!is_valid_mod_id("my@mod"), "ID with @ should be invalid");
        assert!(
            !is_valid_mod_id("my mod"),
            "ID with space should be invalid"
        );
        assert!(
            !is_valid_mod_id("my/mod"),
            "ID with slash should be invalid"
        );
    }

    #[test]
    fn test_is_allowed_mod_file_allowed_extensions() {
        assert!(is_allowed_mod_file("html"), "html should be allowed");
        assert!(is_allowed_mod_file("js"), "js should be allowed");
        assert!(is_allowed_mod_file("css"), "css should be allowed");
        assert!(is_allowed_mod_file("json"), "json should be allowed");
        assert!(is_allowed_mod_file("png"), "png should be allowed");
        assert!(is_allowed_mod_file("jpg"), "jpg should be allowed");
        assert!(is_allowed_mod_file("jpeg"), "jpeg should be allowed");
        assert!(is_allowed_mod_file("webp"), "webp should be allowed");
        assert!(is_allowed_mod_file("svg"), "svg should be allowed");
        assert!(is_allowed_mod_file("gif"), "gif should be allowed");
        assert!(is_allowed_mod_file("woff"), "woff should be allowed");
        assert!(is_allowed_mod_file("woff2"), "woff2 should be allowed");
        assert!(is_allowed_mod_file("ttf"), "ttf should be allowed");
        assert!(is_allowed_mod_file("otf"), "otf should be allowed");
        assert!(is_allowed_mod_file("txt"), "txt should be allowed");
        assert!(is_allowed_mod_file("md"), "md should be allowed");
    }

    #[test]
    fn test_is_allowed_mod_file_case_insensitive() {
        assert!(
            is_allowed_mod_file("HTML"),
            "HTML uppercase should be allowed"
        );
        assert!(is_allowed_mod_file("Js"), "Js mixed case should be allowed");
        assert!(
            is_allowed_mod_file("JSON"),
            "JSON uppercase should be allowed"
        );
    }

    #[test]
    fn test_is_allowed_mod_file_disallowed_extensions() {
        assert!(!is_allowed_mod_file("exe"), "exe should not be allowed");
        assert!(!is_allowed_mod_file("sh"), "sh should not be allowed");
        assert!(!is_allowed_mod_file("bat"), "bat should not be allowed");
        assert!(!is_allowed_mod_file("dll"), "dll should not be allowed");
        assert!(!is_allowed_mod_file("so"), "so should not be allowed");
        assert!(!is_allowed_mod_file("zip"), "zip should not be allowed");
    }

    #[test]
    fn test_is_allowed_mod_file_empty_extension() {
        assert!(
            !is_allowed_mod_file(""),
            "empty extension should not be allowed"
        );
    }
}
