use crate::ai::context::AIOrchestrator;
use crate::characters::catalog::{
    validate_package_directory as validate_catalog_package_directory, CharacterCatalog,
};
use crate::characters::instance_resource::{
    instance_avatar_reference, parse_instance_avatar_reference, validate_avatar_bytes,
    validate_instance_id, MAX_INSTANCE_AVATAR_BYTES,
};
use crate::characters::{validate_package_content, PackageContentEntry};
use crate::error::KokoroError;
use crate::registry::client::verify_registry_entry_archive;
use crate::registry::manifest::{RegistryEntry, OFFICIAL_REGISTRY_URL};
use semver::Version;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Acquire, Row, SqliteConnection, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use tauri::AppHandle;
use tauri::Manager;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

// pattern: Imperative Shell

/// All JSON config filenames we back up.
const CONFIG_FILES: &[&str] = &[
    "llm_config.json",
    "tts_config.json",
    "stt_config.json",
    "vision_config.json",
    "imagegen_config.json",
    "mcp_servers.json",
    "bot_config.json",
    "telegram_config.json",
    "jailbreak_prompt.json",
    "proactive_enabled.json",
    "memory_system_config.json",
    "memory_upgrade_config.json",
    "emotion_state.json",
    "context_settings.json",
    "current_conversation_id.json",
    "user_profile.json",
];

pub(crate) const MAX_BACKUP_RESOURCE_PACKAGES: usize = 64;
pub(crate) const MAX_BACKUP_RESOURCE_FILES: usize = 2_048;
pub(crate) const MAX_BACKUP_RESOURCE_BYTES: u64 = 512 * 1024 * 1024;

// ── Types ────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub version: String,
    pub created_at: String,
    pub app_version: String,
    #[serde(default)]
    pub includes_character_resources: bool,
}

#[derive(Clone, Copy, Debug, Default, Deserialize)]
pub struct ExportOptions {
    #[serde(default)]
    pub include_character_resources: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolvedCharacterPackage {
    pub package_dir: PathBuf,
    pub avatar_path: Option<PathBuf>,
}

pub trait CharacterPackageResolver: Send + Sync {
    fn resolve_exact(
        &self,
        template_id: &str,
        template_version: &str,
    ) -> Result<Option<ResolvedCharacterPackage>, String>;

    fn resolve_instance_avatar(&self, _instance_id: &str) -> Result<Option<String>, String> {
        Ok(None)
    }
}

pub struct LocalCatalogPackageResolver {
    root: PathBuf,
}

impl LocalCatalogPackageResolver {
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }
}

impl CharacterPackageResolver for LocalCatalogPackageResolver {
    fn resolve_exact(
        &self,
        template_id: &str,
        template_version: &str,
    ) -> Result<Option<ResolvedCharacterPackage>, String> {
        let package_dir = self.root.join(template_id).join(template_version);
        if !package_dir.is_dir() {
            return Ok(None);
        }
        let engine_version = Version::parse(env!("CARGO_PKG_VERSION"))
            .map_err(|error| format!("invalid engine version: {error}"))?;
        let manifest = validate_catalog_package_directory(&package_dir, &engine_version)
            .map_err(|error| format!("invalid character package: {error}"))?;
        if manifest.id != template_id || manifest.version != template_version {
            return Err("character package does not match requested exact version".to_string());
        }
        Ok(Some(ResolvedCharacterPackage {
            package_dir,
            avatar_path: manifest.avatar.map(PathBuf::from),
        }))
    }

    fn resolve_instance_avatar(&self, instance_id: &str) -> Result<Option<String>, String> {
        validate_instance_id(instance_id).map_err(|error| error.to_string())?;
        let Some(app_data) = self.root.parent() else {
            return Ok(None);
        };
        let avatar = app_data
            .join("character-instance-resources")
            .join(instance_id)
            .join("avatar.png");
        let directory = avatar
            .parent()
            .ok_or_else(|| "managed character avatar has no resource directory".to_string())?;
        match fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.file_type().is_dir() && !metadata.file_type().is_symlink() => {
            }
            Ok(_) => {
                return Err("managed character avatar directory is unsafe".to_string());
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        }
        match fs::symlink_metadata(&avatar) {
            Ok(metadata)
                if metadata.file_type().is_file() && !metadata.file_type().is_symlink() =>
            {
                let bytes = fs::read(&avatar).map_err(|error| error.to_string())?;
                validate_avatar_bytes(&bytes).map_err(|error| error.to_string())?;
                Ok(Some(
                    instance_avatar_reference(instance_id).map_err(|error| error.to_string())?,
                ))
            }
            Ok(_) => Err("managed character avatar is not a regular file".to_string()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(error) => Err(error.to_string()),
        }
    }
}

/// Resolves missing data-only restore packages from the canonical registry.
/// Network failures and missing versions intentionally return `None`, allowing
/// the restored instance to retain its built-in presentation fallback. Any
/// downloaded bytes still pass the exact registry checksum, manifest, and
/// engine-compatibility checks before they are staged into the local catalog.
pub(crate) struct OfficialRegistryPackageResolver {
    root: PathBuf,
}

impl OfficialRegistryPackageResolver {
    pub(crate) fn new(root: PathBuf) -> Self {
        Self { root }
    }

    pub(crate) async fn hydrate_exact(
        &self,
        template_id: &str,
        template_version: &str,
    ) -> Result<Option<ResolvedCharacterPackage>, String> {
        let index = match crate::commands::registry::fetch_index(OFFICIAL_REGISTRY_URL).await {
            Ok(index) => index,
            Err(error) => {
                tracing::warn!(
                    target: "backup",
                    "official character package registry unavailable: {error}"
                );
                return Ok(None);
            }
        };
        let Some(entry) = index.entries.iter().find(|entry| {
            entry.content_type == "character"
                && entry.id == template_id
                && entry.version == template_version
        }) else {
            return Ok(None);
        };
        let bytes = match crate::commands::registry::fetch_bytes(&entry.download_url).await {
            Ok(bytes) => bytes,
            Err(error) => {
                tracing::warn!(
                    target: "backup",
                    package = %format!("{template_id}@{template_version}"),
                    "official character package unavailable: {error}"
                );
                return Ok(None);
            }
        };
        stage_verified_official_package(&self.root, &bytes, entry)
            .map(Some)
            .map_err(|error| format!("failed to validate official character package: {error}"))
    }
}

impl CharacterPackageResolver for OfficialRegistryPackageResolver {
    fn resolve_exact(
        &self,
        template_id: &str,
        template_version: &str,
    ) -> Result<Option<ResolvedCharacterPackage>, String> {
        LocalCatalogPackageResolver::new(self.root.clone())
            .resolve_exact(template_id, template_version)
    }

    fn resolve_instance_avatar(&self, instance_id: &str) -> Result<Option<String>, String> {
        LocalCatalogPackageResolver::new(self.root.clone()).resolve_instance_avatar(instance_id)
    }
}

pub(crate) fn stage_verified_official_package(
    root: &Path,
    bytes: &[u8],
    entry: &RegistryEntry,
) -> Result<ResolvedCharacterPackage, String> {
    let engine_version = Version::parse(env!("CARGO_PKG_VERSION"))
        .map_err(|error| format!("invalid engine version: {error}"))?;
    let verified = verify_registry_entry_archive(bytes, entry, &engine_version)
        .map_err(|error| format!("{error}"))?;
    let catalog = CharacterCatalog::new(root.to_path_buf(), engine_version);
    let installed = catalog
        .install_zip(Cursor::new(verified.bytes))
        .map_err(|error| format!("failed to stage official character package: {error}"))?;
    if installed.manifest.id != entry.id || installed.manifest.version != entry.version {
        return Err(
            "staged official package does not match the requested exact version".to_string(),
        );
    }
    Ok(ResolvedCharacterPackage {
        package_dir: installed.package_dir,
        avatar_path: installed.manifest.avatar.map(PathBuf::from),
    })
}

#[derive(Debug, PartialEq, Eq)]
pub struct BackupArchiveInspection {
    pub has_character_resources: bool,
    pub includes_provider_credentials: bool,
}

#[derive(Debug, Serialize)]
pub struct BackupStats {
    pub memories: i64,
    pub conversations: i64,
    pub messages: i64,
    pub configs: usize,
}

#[derive(Debug, Serialize)]
pub struct ExportResult {
    pub path: String,
    pub size_bytes: u64,
    pub stats: BackupStats,
}

#[derive(Debug, Serialize)]
pub struct ImportPreview {
    pub manifest: BackupManifest,
    pub has_database: bool,
    pub has_configs: bool,
    pub config_files: Vec<String>,
    pub stats: BackupStats,
}

#[derive(Debug, Deserialize)]
pub struct ImportOptions {
    pub import_database: bool,
    pub import_configs: bool,
    pub conflict_strategy: ConflictStrategy,
    pub target_character_id: Option<String>,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ConflictStrategy {
    Skip,
    Overwrite,
}

impl ConflictStrategy {
    fn as_str(self) -> &'static str {
        match self {
            Self::Skip => "skip",
            Self::Overwrite => "overwrite",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct ImportResult {
    pub imported_memories: i64,
    pub imported_conversations: i64,
    pub imported_configs: usize,
    pub imported_characters: i64,
    pub characters_json: Option<String>,
    pub debug_log: Vec<String>,
}

// ── Helpers ──────────────────────────────────────────

fn app_data_dir(app: &AppHandle) -> Result<PathBuf, KokoroError> {
    app.path()
        .app_data_dir()
        .map_err(|e| KokoroError::Internal(format!("Failed to resolve app data dir: {}", e)))
}

fn db_path(app_data: &Path) -> PathBuf {
    app_data.join("kokoro.db")
}

pub fn db_path_pub(app_data: &Path) -> PathBuf {
    db_path(app_data)
}

/// Validate that a filename from a ZIP entry is safe (no path traversal).
/// RAII 临时目录守卫：离开作用域时自动删除目录，确保错误路径也能清理
struct TempDirGuard(std::path::PathBuf);

impl TempDirGuard {
    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDirGuard {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn create_scoped_temp_dir(prefix: &str) -> Result<TempDirGuard, KokoroError> {
    let path = std::env::temp_dir().join(format!("{prefix}-{}", Uuid::new_v4()));
    fs::create_dir(&path).map_err(KokoroError::from)?;
    Ok(TempDirGuard(path))
}

pub(crate) fn stage_backup_configs(
    backup_path: &Path,
) -> Result<Vec<(String, String)>, KokoroError> {
    let file = fs::File::open(backup_path).map_err(KokoroError::from)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| KokoroError::Validation(format!("invalid backup archive: {error}")))?;
    let allowed: HashSet<&str> = CONFIG_FILES.iter().copied().collect();
    let mut seen = HashSet::new();
    let mut staged = Vec::new();
    for index in 0..archive.len() {
        let mut entry = archive
            .by_index(index)
            .map_err(|error| KokoroError::Validation(format!("invalid backup entry: {error}")))?;
        let name = entry.name().to_string();
        let Some(filename) = name.strip_prefix("configs/") else {
            continue;
        };
        if filename.is_empty()
            || filename.contains('/')
            || filename.contains('\\')
            || entry.is_dir()
        {
            return Err(KokoroError::Validation(format!(
                "config path must contain a single filename: {name}"
            )));
        }
        if !allowed.contains(filename) {
            return Err(KokoroError::Validation(format!(
                "unknown config filename: {filename}"
            )));
        }
        if !seen.insert(filename.to_string()) {
            return Err(KokoroError::Validation(format!(
                "duplicate config filename: {filename}"
            )));
        }
        let mut content = String::new();
        entry.read_to_string(&mut content).map_err(|error| {
            KokoroError::Validation(format!("invalid UTF-8 config {filename}: {error}"))
        })?;
        serde_json::from_str::<serde_json::Value>(&content).map_err(|error| {
            KokoroError::Validation(format!("invalid JSON config {filename}: {error}"))
        })?;
        staged.push((filename.to_string(), content));
    }
    Ok(staged)
}

pub(crate) fn validate_backup_resource_totals(
    package_count: usize,
    file_count: usize,
    total_uncompressed_bytes: u64,
) -> Result<(), KokoroError> {
    if package_count > MAX_BACKUP_RESOURCE_PACKAGES {
        return Err(KokoroError::Validation(format!(
            "character resource package count exceeds limit of {MAX_BACKUP_RESOURCE_PACKAGES}"
        )));
    }
    if file_count > MAX_BACKUP_RESOURCE_FILES {
        return Err(KokoroError::Validation(format!(
            "character resource file count exceeds limit of {MAX_BACKUP_RESOURCE_FILES}"
        )));
    }
    if total_uncompressed_bytes > MAX_BACKUP_RESOURCE_BYTES {
        return Err(KokoroError::Validation(format!(
            "character resource uncompressed byte limit exceeds {MAX_BACKUP_RESOURCE_BYTES}"
        )));
    }
    Ok(())
}

fn replace_configs_atomically(
    app_data: &Path,
    configs: &[(String, String)],
) -> Result<ConfigReplacementGuard, KokoroError> {
    for (filename, _) in configs {
        let target = app_data.join(filename);
        if target.exists() {
            let metadata = fs::symlink_metadata(&target).map_err(KokoroError::from)?;
            if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                return Err(KokoroError::Validation(format!(
                    "config target is not a regular non-symlink file: {filename}"
                )));
            }
        }
    }
    let token = Uuid::new_v4();
    let mut plans: Vec<(PathBuf, PathBuf, PathBuf, bool, bool)> = Vec::with_capacity(configs.len());
    for (filename, content) in configs {
        let target = app_data.join(filename);
        let temporary = app_data.join(format!(".{filename}.{token}.import"));
        let backup = app_data.join(format!(".{filename}.{token}.backup"));
        if let Err(error) = fs::write(&temporary, content) {
            for (_, staged, _, _, _) in &plans {
                let _ = fs::remove_file(staged);
            }
            return Err(error.into());
        }
        plans.push((target, temporary, backup, false, false));
    }

    let result = (|| -> Result<(), std::io::Error> {
        for (target, temporary, backup, had_original, was_installed) in &mut plans {
            if target.exists() {
                fs::rename(&*target, &*backup)?;
                *had_original = true;
            }
            if let Err(error) = fs::rename(&*temporary, &*target) {
                if *had_original {
                    let _ = fs::rename(&*backup, &*target);
                }
                return Err(error);
            }
            *was_installed = true;
        }
        Ok(())
    })();

    if let Err(error) = result {
        for (target, temporary, backup, had_original, was_installed) in plans.iter().rev() {
            let _ = fs::remove_file(temporary);
            if *was_installed {
                let _ = fs::remove_file(target);
            }
            if *had_original {
                let _ = fs::rename(backup, target);
            }
        }
        return Err(error.into());
    }
    Ok(ConfigReplacementGuard {
        plans,
        is_armed: true,
    })
}

struct ConfigReplacementGuard {
    plans: Vec<(PathBuf, PathBuf, PathBuf, bool, bool)>,
    is_armed: bool,
}

impl ConfigReplacementGuard {
    fn imported_count(&self) -> usize {
        self.plans.len()
    }

    fn disarm(&mut self) {
        if !self.is_armed {
            return;
        }
        for (_, _, backup, had_original, _) in &self.plans {
            if *had_original {
                let _ = fs::remove_file(backup);
            }
        }
        self.is_armed = false;
    }
}

impl Drop for ConfigReplacementGuard {
    fn drop(&mut self) {
        if !self.is_armed {
            return;
        }
        for (target, temporary, backup, had_original, was_installed) in self.plans.iter().rev() {
            let _ = fs::remove_file(temporary);
            if *was_installed {
                let _ = fs::remove_file(target);
            }
            if *had_original {
                let _ = fs::rename(backup, target);
            }
        }
    }
}

struct ResourcePromotionGuard {
    created_targets: Vec<PathBuf>,
    replaced_targets: Vec<(PathBuf, PathBuf)>,
    is_armed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct StagedCharacterResources {
    packages: Vec<(String, String)>,
    instance_ids: Vec<String>,
}

impl ResourcePromotionGuard {
    fn empty() -> Self {
        Self {
            created_targets: Vec::new(),
            replaced_targets: Vec::new(),
            is_armed: true,
        }
    }

    fn disarm(&mut self) {
        if !self.is_armed {
            return;
        }
        for (_, backup) in &self.replaced_targets {
            let _ = fs::remove_dir_all(backup);
        }
        self.is_armed = false;
    }
}

impl Drop for ResourcePromotionGuard {
    fn drop(&mut self) {
        if self.is_armed {
            for target in self.created_targets.iter().rev() {
                let _ = fs::remove_dir_all(target);
            }
            for (target, backup) in self.replaced_targets.iter().rev() {
                let _ = fs::remove_dir_all(target);
                let _ = fs::rename(backup, target);
            }
        }
    }
}

fn promote_staged_resources(
    staging_root: &Path,
    catalog_root: &Path,
    staged: &StagedCharacterResources,
    conflict_strategy: ConflictStrategy,
) -> Result<ResourcePromotionGuard, KokoroError> {
    let mut guard = ResourcePromotionGuard::empty();
    for (id, version) in &staged.packages {
        let source = staging_root.join(id).join(version);
        let target = catalog_root.join(id).join(version);
        if target.exists() {
            let metadata = fs::symlink_metadata(&target)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(KokoroError::Validation(format!(
                    "installed character package target {id}@{version} is unsafe"
                )));
            }
            if conflict_strategy == ConflictStrategy::Skip {
                continue;
            }
            let backup = target.with_file_name(format!(
                ".{}.import-backup-{}",
                target
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("avatar"),
                Uuid::new_v4()
            ));
            fs::rename(&target, &backup).map_err(KokoroError::from)?;
            if let Err(error) = fs::rename(&source, &target) {
                let _ = fs::rename(&backup, &target);
                return Err(error.into());
            }
            guard.replaced_targets.push((target, backup));
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(KokoroError::from)?;
        }
        fs::rename(&source, &target).map_err(KokoroError::from)?;
        guard.created_targets.push(target);
    }
    let app_data = catalog_root.parent().ok_or_else(|| {
        KokoroError::Validation("character catalog has no app-data parent".to_string())
    })?;
    for instance_id in &staged.instance_ids {
        let source = staging_root.join(".instances").join(instance_id);
        let target = app_data
            .join("character-instance-resources")
            .join(instance_id);
        if target.exists() {
            let metadata = fs::symlink_metadata(&target)?;
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return Err(KokoroError::Validation(format!(
                    "installed managed avatar target for {instance_id} is unsafe"
                )));
            }
            if conflict_strategy == ConflictStrategy::Skip {
                continue;
            }
            let backup =
                target.with_file_name(format!(".{}.import-backup-{}", instance_id, Uuid::new_v4()));
            fs::rename(&target, &backup).map_err(KokoroError::from)?;
            if let Err(error) = fs::rename(&source, &target) {
                let _ = fs::rename(&backup, &target);
                return Err(error.into());
            }
            guard.replaced_targets.push((target, backup));
            continue;
        }
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent).map_err(KokoroError::from)?;
        }
        fs::rename(source, &target).map_err(KokoroError::from)?;
        guard.created_targets.push(target);
    }
    Ok(guard)
}

fn package_directory_entries(root: &Path) -> Result<Vec<PackageContentEntry>, String> {
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let file_type = entry.file_type().map_err(|error| error.to_string())?;
            let path = entry.path();
            let relative = path
                .strip_prefix(root)
                .map_err(|error| error.to_string())?
                .to_path_buf();
            entries.push(PackageContentEntry {
                path: relative,
                uncompressed_size: if file_type.is_file() {
                    entry.metadata().map_err(|error| error.to_string())?.len()
                } else {
                    0
                },
                is_directory: file_type.is_dir(),
            });
            if file_type.is_dir() {
                pending.push(path);
            }
        }
    }
    Ok(entries)
}

pub(crate) fn inspect_backup_archive(path: &Path) -> Result<BackupArchiveInspection, KokoroError> {
    let file = fs::File::open(path).map_err(KokoroError::from)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|error| KokoroError::Validation(format!("invalid backup archive: {error}")))?;
    let mut has_character_resources = false;
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| KokoroError::Validation(format!("invalid backup entry: {error}")))?;
        if entry.name().starts_with("character-resources/")
            || entry.name().starts_with("character-instance-resources/")
        {
            has_character_resources = true;
        }
    }
    Ok(BackupArchiveInspection {
        has_character_resources,
        // Provider credentials are deliberately outside both backup modes.
        includes_provider_credentials: false,
    })
}

fn should_stage_character_resources(
    options: &ImportOptions,
    inspection: &BackupArchiveInspection,
) -> bool {
    options.import_database && inspection.has_character_resources
}

pub(crate) fn stage_character_resources(
    backup_path: &Path,
    staging_root: &Path,
) -> Result<StagedCharacterResources, KokoroError> {
    if staging_root.exists() {
        fs::remove_dir_all(staging_root).map_err(KokoroError::from)?;
    }
    fs::create_dir_all(staging_root).map_err(KokoroError::from)?;
    let result = (|| -> Result<StagedCharacterResources, KokoroError> {
        let file = fs::File::open(backup_path).map_err(KokoroError::from)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| KokoroError::Validation(format!("invalid backup archive: {error}")))?;
        let mut packages: HashMap<(String, String), Vec<PackageContentEntry>> = HashMap::new();
        let mut instance_ids = HashSet::new();
        let mut resource_file_count = 0_usize;
        let mut resource_uncompressed_bytes = 0_u64;

        for index in 0..archive.len() {
            let entry = archive.by_index(index).map_err(|error| {
                KokoroError::Validation(format!("invalid backup resource entry: {error}"))
            })?;
            let name = entry.name();
            if let Some(relative) = name.strip_prefix("character-instance-resources/") {
                let components: Vec<&str> = relative.split('/').collect();
                if entry.is_dir()
                    || components.len() != 2
                    || components[1] != "avatar.png"
                    || validate_instance_id(components[0]).is_err()
                    || !instance_ids.insert(components[0].to_string())
                {
                    return Err(KokoroError::Validation(format!(
                        "invalid or duplicate character instance resource path: {name}"
                    )));
                }
                if entry.size() > MAX_INSTANCE_AVATAR_BYTES as u64 {
                    return Err(KokoroError::Validation(
                        "character avatar must be a PNG no larger than 16 MiB".to_string(),
                    ));
                }
                resource_file_count = resource_file_count.checked_add(1).ok_or_else(|| {
                    KokoroError::Validation("character resource file count overflow".to_string())
                })?;
                resource_uncompressed_bytes = resource_uncompressed_bytes
                    .checked_add(entry.size())
                    .ok_or_else(|| {
                        KokoroError::Validation(
                            "character resource uncompressed byte count overflow".to_string(),
                        )
                    })?;
                continue;
            }
            if !name.starts_with("character-resources/") {
                continue;
            }
            let components: Vec<&str> = name.split('/').collect();
            if components.len() < 4
                || components[1].is_empty()
                || components[2].is_empty()
                || components[3..]
                    .iter()
                    .any(|part| part.is_empty() && !entry.is_dir())
            {
                return Err(KokoroError::Validation(format!(
                    "invalid character resource path: {name}"
                )));
            }
            let relative = components[3..].join("/");
            let relative_path = PathBuf::from(relative.trim_end_matches('/'));
            crate::characters::manifest::validate_package_path(&relative_path)
                .map_err(|error| KokoroError::Validation(error.to_string()))?;
            if !entry.is_dir() {
                resource_file_count = resource_file_count.checked_add(1).ok_or_else(|| {
                    KokoroError::Validation("character resource file count overflow".to_string())
                })?;
                resource_uncompressed_bytes = resource_uncompressed_bytes
                    .checked_add(entry.size())
                    .ok_or_else(|| {
                        KokoroError::Validation(
                            "character resource uncompressed byte count overflow".to_string(),
                        )
                    })?;
            }
            packages
                .entry((components[1].to_string(), components[2].to_string()))
                .or_default()
                .push(PackageContentEntry {
                    path: relative_path,
                    uncompressed_size: entry.size(),
                    is_directory: entry.is_dir(),
                });
        }

        validate_backup_resource_totals(
            packages.len() + instance_ids.len(),
            resource_file_count,
            resource_uncompressed_bytes,
        )?;

        for entries in packages.values() {
            validate_package_content(entries)
                .map_err(|error| KokoroError::Validation(error.to_string()))?;
        }

        for index in 0..archive.len() {
            let mut entry = archive.by_index(index).map_err(|error| {
                KokoroError::Validation(format!("invalid backup resource entry: {error}"))
            })?;
            if entry.name().starts_with("character-instance-resources/") {
                let relative = entry
                    .enclosed_name()
                    .ok_or_else(|| {
                        KokoroError::Validation(
                            "unsafe character instance resource path".to_string(),
                        )
                    })?
                    .strip_prefix("character-instance-resources")
                    .map_err(|_| {
                        KokoroError::Validation(
                            "unsafe character instance resource path".to_string(),
                        )
                    })?
                    .to_path_buf();
                let destination = staging_root.join(".instances").join(relative);
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(KokoroError::from)?;
                }
                let mut output = fs::File::create(destination).map_err(KokoroError::from)?;
                std::io::copy(&mut entry, &mut output).map_err(KokoroError::from)?;
                continue;
            }
            if !entry.name().starts_with("character-resources/") {
                continue;
            }
            let relative = entry
                .enclosed_name()
                .ok_or_else(|| KokoroError::Validation("unsafe backup resource path".to_string()))?
                .strip_prefix("character-resources")
                .map_err(|_| KokoroError::Validation("unsafe backup resource path".to_string()))?
                .to_path_buf();
            let destination = staging_root.join(relative);
            if entry.is_dir() {
                fs::create_dir_all(&destination).map_err(KokoroError::from)?;
            } else {
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent).map_err(KokoroError::from)?;
                }
                let mut output = fs::File::create(&destination).map_err(KokoroError::from)?;
                std::io::copy(&mut entry, &mut output).map_err(KokoroError::from)?;
            }
        }

        let resolver = LocalCatalogPackageResolver::new(staging_root.to_path_buf());
        let mut validated = Vec::new();
        for (id, version) in packages.keys() {
            if resolver
                .resolve_exact(id, version)
                .map_err(KokoroError::Validation)?
                .is_none()
            {
                return Err(KokoroError::Validation(format!(
                    "missing staged character package {id}@{version}"
                )));
            }
            validated.push((id.clone(), version.clone()));
        }
        for instance_id in &instance_ids {
            let avatar = staging_root
                .join(".instances")
                .join(instance_id)
                .join("avatar.png");
            let metadata = fs::symlink_metadata(&avatar).map_err(KokoroError::from)?;
            if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
                return Err(KokoroError::Validation(format!(
                    "invalid managed avatar for character {instance_id}"
                )));
            }
            let bytes = fs::read(&avatar).map_err(KokoroError::from)?;
            validate_avatar_bytes(&bytes)
                .map_err(|error| KokoroError::Validation(error.to_string()))?;
        }
        Ok(StagedCharacterResources {
            packages: validated,
            instance_ids: instance_ids.into_iter().collect(),
        })
    })();

    if result.is_err() {
        let _ = fs::remove_dir_all(staging_root);
    }
    result
}

#[derive(Debug)]
struct PreparedCharacterRow {
    id: String,
    name: String,
    persona: String,
    user_nickname: String,
    source_format: String,
    created_at: i64,
    updated_at: i64,
    template_id: Option<String>,
    template_version: Option<String>,
    template_snapshot_json: Option<String>,
    description: String,
    avatar_path: Option<String>,
    greeting: String,
    greeting_consumed_at: Option<i64>,
    greeting_message_id: Option<i64>,
    example_dialogue: String,
    runtime_profile_json: String,
    user_modified_at: Option<i64>,
}

async fn prepare_character_rows(
    source: &SqlitePool,
    resolver: &dyn CharacterPackageResolver,
) -> Result<Vec<PreparedCharacterRow>, KokoroError> {
    let source_has_table: Option<String> = sqlx::query_scalar(
        "SELECT name FROM sqlite_master WHERE type='table' AND name='characters'",
    )
    .fetch_optional(source)
    .await?;
    if source_has_table.is_none() {
        return Ok(Vec::new());
    }

    let columns: Vec<String> = sqlx::query("PRAGMA table_info(characters)")
        .fetch_all(source)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    let has_column = |name: &str| columns.iter().any(|column| column == name);
    let optional_text = |name: &str| {
        if has_column(name) {
            name.to_string()
        } else {
            format!("NULL AS {name}")
        }
    };
    let text_default = |name: &str, default: &str| {
        if has_column(name) {
            name.to_string()
        } else {
            format!("'{default}' AS {name}")
        }
    };
    let optional_integer = |name: &str| {
        if has_column(name) {
            name.to_string()
        } else {
            format!("NULL AS {name}")
        }
    };
    let greeting_consumed = if has_column("greeting_consumed_at") {
        "greeting_consumed_at".to_string()
    } else {
        "updated_at AS greeting_consumed_at".to_string()
    };
    let select = format!(
        "SELECT id, name, persona, user_nickname, source_format, created_at, updated_at, {}, {}, {}, {}, {}, {}, {}, {}, {}, {}, {} FROM characters",
        optional_text("template_id"),
        optional_text("template_version"),
        optional_text("template_snapshot_json"),
        text_default("description", ""),
        optional_text("avatar_path"),
        text_default("greeting", ""),
        greeting_consumed,
        optional_integer("greeting_message_id"),
        text_default("example_dialogue", ""),
        text_default("runtime_profile_json", "{}"),
        optional_integer("user_modified_at"),
    );
    let rows = sqlx::query(&select).fetch_all(source).await?;
    let mut prepared = Vec::with_capacity(rows.len());
    for row in rows {
        let id: String = row.try_get("id")?;
        let template_id: Option<String> = row.try_get("template_id")?;
        let template_version: Option<String> = row.try_get("template_version")?;
        let stored_avatar_path: Option<String> = row.try_get("avatar_path")?;
        let managed_avatar = stored_avatar_path
            .as_deref()
            .and_then(parse_instance_avatar_reference)
            .filter(|resource_id| *resource_id == id.as_str())
            .map(|resource_id| resolver.resolve_instance_avatar(resource_id))
            .transpose()
            .map_err(KokoroError::Validation)?
            .flatten();
        let avatar_path = if managed_avatar.is_some() {
            managed_avatar
        } else {
            match (&template_id, &template_version) {
                (Some(template_id), Some(template_version)) => resolver
                    .resolve_exact(template_id, template_version)
                    .map_err(KokoroError::Validation)?
                    .and_then(|package| {
                        package.avatar_path.map(|relative| {
                            package
                                .package_dir
                                .join(relative)
                                .to_string_lossy()
                                .into_owned()
                        })
                    }),
                _ => None,
            }
        };
        prepared.push(PreparedCharacterRow {
            id,
            name: row.try_get("name")?,
            persona: row.try_get("persona")?,
            user_nickname: row.try_get("user_nickname")?,
            source_format: row.try_get("source_format")?,
            created_at: row.try_get("created_at")?,
            updated_at: row.try_get("updated_at")?,
            template_id,
            template_version,
            template_snapshot_json: row.try_get("template_snapshot_json")?,
            description: row.try_get("description")?,
            avatar_path,
            greeting: row.try_get("greeting")?,
            greeting_consumed_at: row.try_get("greeting_consumed_at")?,
            greeting_message_id: row.try_get("greeting_message_id")?,
            example_dialogue: row.try_get("example_dialogue")?,
            runtime_profile_json: row.try_get("runtime_profile_json")?,
            user_modified_at: row.try_get("user_modified_at")?,
        });
    }
    Ok(prepared)
}

async fn apply_character_rows(
    transaction: &mut sqlx::SqliteConnection,
    rows: Vec<PreparedCharacterRow>,
    conflict_strategy: &str,
) -> Result<i64, KokoroError> {
    let mut restored = 0_i64;
    for row in rows {
        if conflict_strategy == "overwrite" {
            sqlx::query("DELETE FROM characters WHERE id = ?")
                .bind(&row.id)
                .execute(&mut *transaction)
                .await?;
        }
        let result = sqlx::query(
            "INSERT OR IGNORE INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at, template_id, template_version, template_snapshot_json, description, avatar_path, greeting, greeting_consumed_at, greeting_message_id, example_dialogue, runtime_profile_json, user_modified_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&row.id)
        .bind(row.name)
        .bind(row.persona)
        .bind(row.user_nickname)
        .bind(row.source_format)
        .bind(row.created_at)
        .bind(row.updated_at)
        .bind(row.template_id)
        .bind(row.template_version)
        .bind(row.template_snapshot_json)
        .bind(row.description)
        .bind(row.avatar_path)
        .bind(row.greeting)
        .bind(row.greeting_consumed_at)
        .bind(row.greeting_message_id)
        .bind(row.example_dialogue)
        .bind(row.runtime_profile_json)
        .bind(row.user_modified_at)
        .execute(&mut *transaction)
        .await?;
        restored += result.rows_affected() as i64;
    }
    Ok(restored)
}

#[cfg(test)]
pub(crate) async fn restore_character_rows(
    target: &SqlitePool,
    source: &SqlitePool,
    conflict_strategy: &str,
    resolver: &dyn CharacterPackageResolver,
) -> Result<i64, KokoroError> {
    let rows = prepare_character_rows(source, resolver).await?;
    let mut transaction = target.begin().await?;
    let restored = apply_character_rows(&mut transaction, rows, conflict_strategy).await?;
    transaction.commit().await?;
    Ok(restored)
}

async fn detach_import_database_best_effort(connection: &mut SqliteConnection) -> bool {
    match sqlx::query("DETACH DATABASE import_db")
        .execute(connection)
        .await
    {
        Ok(_) => true,
        Err(error) => {
            tracing::warn!(
                target: "backup",
                "import committed but DETACH DATABASE import_db failed: {error}"
            );
            false
        }
    }
}

/// Open a read-only sqlx pool to a given DB file.
async fn open_readonly_pool(path: &Path) -> Result<SqlitePool, KokoroError> {
    let url = format!("sqlite://{}", path.to_string_lossy().replace('\\', "/"));
    let options = SqliteConnectOptions::from_str(&url)
        .map_err(|e| KokoroError::Internal(format!("Invalid DB path: {}", e)))?
        .read_only(true);
    SqlitePool::connect_with(options)
        .await
        .map_err(|e| KokoroError::Database(format!("Failed to open DB: {}", e)))
}

async fn open_import_pool_without_orchestrator(app_data: &Path) -> Result<SqlitePool, KokoroError> {
    let db = db_path(app_data);
    let db_url = format!("sqlite:///{}", db.to_string_lossy().replace('\\', "/"));
    let orchestrator = AIOrchestrator::new(&db_url).await.map_err(|e| {
        KokoroError::Internal(format!("Failed to init fallback orchestrator DB: {}", e))
    })?;
    Ok(orchestrator.db)
}

async fn resolve_import_pool(app: &AppHandle, app_data: &Path) -> Result<SqlitePool, KokoroError> {
    if let Some(orchestrator) = app.try_state::<AIOrchestrator>() {
        return Ok(orchestrator.db.clone());
    }

    tracing::warn!(
        target: "backup",
        "AIOrchestrator not managed, using fallback pool for import_data"
    );
    open_import_pool_without_orchestrator(app_data).await
}

/// 受限的表名枚举，防止 count_rows 被传入任意字符串
enum CountTable {
    Memories,
    Conversations,
    ConversationMessages,
}

impl CountTable {
    fn as_sql(&self) -> &'static str {
        match self {
            CountTable::Memories => "SELECT COUNT(*) as cnt FROM memories",
            CountTable::Conversations => "SELECT COUNT(*) as cnt FROM conversations",
            CountTable::ConversationMessages => "SELECT COUNT(*) as cnt FROM conversation_messages",
        }
    }
}

/// Count rows in a table via sqlx. Returns 0 on any error.
async fn count_rows(pool: &SqlitePool, table: CountTable) -> i64 {
    sqlx::query(table.as_sql())
        .fetch_one(pool)
        .await
        .and_then(|row| row.try_get::<i64, _>("cnt"))
        .unwrap_or(0)
}

async fn gather_stats(path: &Path) -> BackupStats {
    let pool = match open_readonly_pool(path).await {
        Ok(p) => p,
        Err(_) => {
            return BackupStats {
                memories: 0,
                conversations: 0,
                messages: 0,
                configs: 0,
            }
        }
    };
    let memories = count_rows(&pool, CountTable::Memories).await;
    let conversations = count_rows(&pool, CountTable::Conversations).await;
    let messages = count_rows(&pool, CountTable::ConversationMessages).await;
    pool.close().await;
    BackupStats {
        memories,
        conversations,
        messages,
        configs: 0,
    }
}

async fn write_character_resources<W: Write + Seek>(
    zip: &mut zip::ZipWriter<W>,
    options: SimpleFileOptions,
    app_data: &Path,
    database: &Path,
) -> Result<(), KokoroError> {
    if !database.is_file() {
        return Ok(());
    }
    let pool = open_readonly_pool(database).await?;
    let references = sqlx::query(
        "SELECT DISTINCT template_id, template_version FROM characters \
         WHERE template_id IS NOT NULL AND template_version IS NOT NULL",
    )
    .fetch_all(&pool)
    .await?;
    let instance_avatars = sqlx::query(
        "SELECT id, avatar_path FROM characters \
         WHERE avatar_path LIKE 'character-instance-resource://%/avatar.png'",
    )
    .fetch_all(&pool)
    .await?;
    pool.close().await;
    let resolver = LocalCatalogPackageResolver::new(app_data.join("characters"));
    for row in references {
        let template_id: String = row.try_get("template_id")?;
        let template_version: String = row.try_get("template_version")?;
        let package = resolver
            .resolve_exact(&template_id, &template_version)
            .map_err(KokoroError::Validation)?
            .ok_or_else(|| {
                KokoroError::Validation(format!(
                    "character package {template_id}@{template_version} is not installed"
                ))
            })?;
        let mut entries =
            package_directory_entries(&package.package_dir).map_err(KokoroError::Validation)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        for entry in entries.into_iter().filter(|entry| !entry.is_directory) {
            let source = package.package_dir.join(&entry.path);
            let archive_path = format!(
                "character-resources/{}/{}/{}",
                template_id,
                template_version,
                entry.path.to_string_lossy().replace('\\', "/")
            );
            zip.start_file(&archive_path, options)
                .map_err(|error| KokoroError::Internal(format!("ZIP error: {error}")))?;
            let mut input = fs::File::open(source).map_err(KokoroError::from)?;
            std::io::copy(&mut input, zip).map_err(KokoroError::from)?;
        }
    }
    for row in instance_avatars {
        let instance_id: String = row.try_get("id")?;
        let reference: String = row.try_get("avatar_path")?;
        if parse_instance_avatar_reference(&reference) != Some(instance_id.as_str()) {
            return Err(KokoroError::Validation(format!(
                "invalid managed avatar reference for character {instance_id}"
            )));
        }
        resolver
            .resolve_instance_avatar(&instance_id)
            .map_err(KokoroError::Validation)?
            .ok_or_else(|| {
                KokoroError::Validation(format!(
                    "managed avatar resource for character {instance_id} is missing"
                ))
            })?;
        let source = app_data
            .join("character-instance-resources")
            .join(&instance_id)
            .join("avatar.png");
        zip.start_file(
            format!("character-instance-resources/{instance_id}/avatar.png"),
            options,
        )
        .map_err(|error| KokoroError::Internal(format!("ZIP error: {error}")))?;
        let mut input = fs::File::open(source).map_err(KokoroError::from)?;
        std::io::copy(&mut input, zip).map_err(KokoroError::from)?;
    }
    Ok(())
}

// ── Commands ─────────────────────────────────────────

#[tauri::command]
pub async fn export_data(
    app: AppHandle,
    export_path: String,
    _characters_json: Option<String>,
    options: Option<ExportOptions>,
) -> Result<ExportResult, KokoroError> {
    let app_data = app_data_dir(&app)?;
    let db = db_path(&app_data);

    let out_path = PathBuf::from(&export_path);
    let file = fs::File::create(&out_path).map_err(KokoroError::from)?;
    let mut zip = zip::ZipWriter::new(file);
    let zip_options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1. Gather stats before copying
    let mut stats = gather_stats(&db).await;
    let mut config_count: usize = 0;

    // 2. manifest.json
    let options_value = options.unwrap_or_default();
    let manifest = BackupManifest {
        version: "2".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        includes_character_resources: options_value.include_character_resources,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| KokoroError::Internal(format!("Serialize error: {}", e)))?;
    zip.start_file("manifest.json", zip_options)
        .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
    zip.write_all(manifest_json.as_bytes())
        .map_err(KokoroError::from)?;

    // 3. kokoro.db — fs::copy to temp to avoid WAL lock issues
    if db.exists() {
        let tmp_db = app_data.join("kokoro_backup_tmp.db");
        fs::copy(&db, &tmp_db).map_err(KokoroError::from)?;
        // Also copy WAL/SHM if present so the copy is consistent
        let wal = db.with_extension("db-wal");
        let shm = db.with_extension("db-shm");
        if wal.exists() {
            let _ = fs::copy(&wal, tmp_db.with_extension("db-wal"));
        }
        if shm.exists() {
            let _ = fs::copy(&shm, tmp_db.with_extension("db-shm"));
        }

        // Checkpoint the temp copy to merge WAL into main DB file
        {
            let url = format!("sqlite://{}", tmp_db.to_string_lossy().replace('\\', "/"));
            if let Ok(opts) = SqliteConnectOptions::from_str(&url) {
                if let Ok(pool) = SqlitePool::connect_with(opts).await {
                    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                        .execute(&pool)
                        .await;
                    pool.close().await;
                }
            }
        }

        let mut db_bytes = Vec::new();
        fs::File::open(&tmp_db)
            .map_err(KokoroError::from)?
            .read_to_end(&mut db_bytes)
            .map_err(KokoroError::from)?;

        // Clean up temp files
        let _ = fs::remove_file(&tmp_db);
        let _ = fs::remove_file(tmp_db.with_extension("db-wal"));
        let _ = fs::remove_file(tmp_db.with_extension("db-shm"));

        zip.start_file("kokoro.db", zip_options)
            .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
        zip.write_all(&db_bytes).map_err(KokoroError::from)?;
    }

    // 4. Optional character packages. Character rows themselves are already in SQLite.
    if options_value.include_character_resources {
        write_character_resources(&mut zip, zip_options, &app_data, &db).await?;
    }

    // 5. configs/
    for name in CONFIG_FILES {
        let cfg_path = app_data.join(name);
        if cfg_path.exists() {
            if let Ok(content) = fs::read_to_string(&cfg_path) {
                let entry = format!("configs/{}", name);
                zip.start_file(&entry, zip_options)
                    .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
                zip.write_all(content.as_bytes())
                    .map_err(KokoroError::from)?;
                config_count += 1;
            }
        }
    }

    zip.finish()
        .map_err(|e| KokoroError::Internal(format!("ZIP finish error: {}", e)))?;

    let size_bytes = fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
    stats.configs = config_count;

    tracing::info!(
        target: "backup",
        "[Backup] Exported to {} ({} bytes, {} memories, {} conversations, {} configs)",
        export_path, size_bytes, stats.memories, stats.conversations, stats.configs
    );

    Ok(ExportResult {
        path: export_path,
        size_bytes,
        stats,
    })
}

/// 核心导出逻辑，供自动备份模块复用（不需要 AppHandle）
pub async fn export_data_to_path(
    app_data: &Path,
    out_path: &Path,
    _characters_json: Option<String>,
) -> Result<ExportResult, KokoroError> {
    let db = db_path(app_data);

    let file = fs::File::create(out_path).map_err(KokoroError::from)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let mut stats = gather_stats(&db).await;
    let mut config_count: usize = 0;

    let manifest = BackupManifest {
        version: "1".to_string(),
        created_at: chrono::Utc::now().to_rfc3339(),
        app_version: env!("CARGO_PKG_VERSION").to_string(),
        includes_character_resources: false,
    };
    let manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|e| KokoroError::Internal(format!("Serialize error: {}", e)))?;
    zip.start_file("manifest.json", options)
        .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
    zip.write_all(manifest_json.as_bytes())
        .map_err(KokoroError::from)?;

    if db.exists() {
        let tmp_db = app_data.join("kokoro_autobackup_tmp.db");
        fs::copy(&db, &tmp_db).map_err(KokoroError::from)?;
        let wal = db.with_extension("db-wal");
        let shm = db.with_extension("db-shm");
        if wal.exists() {
            let _ = fs::copy(&wal, tmp_db.with_extension("db-wal"));
        }
        if shm.exists() {
            let _ = fs::copy(&shm, tmp_db.with_extension("db-shm"));
        }
        {
            let url = format!("sqlite://{}", tmp_db.to_string_lossy().replace('\\', "/"));
            if let Ok(opts) = SqliteConnectOptions::from_str(&url) {
                if let Ok(pool) = SqlitePool::connect_with(opts).await {
                    let _ = sqlx::query("PRAGMA wal_checkpoint(TRUNCATE)")
                        .execute(&pool)
                        .await;
                    pool.close().await;
                }
            }
        }
        let mut db_bytes = Vec::new();
        fs::File::open(&tmp_db)
            .map_err(KokoroError::from)?
            .read_to_end(&mut db_bytes)
            .map_err(KokoroError::from)?;
        let _ = fs::remove_file(&tmp_db);
        let _ = fs::remove_file(tmp_db.with_extension("db-wal"));
        let _ = fs::remove_file(tmp_db.with_extension("db-shm"));
        zip.start_file("kokoro.db", options)
            .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
        zip.write_all(&db_bytes).map_err(KokoroError::from)?;
    }

    for name in CONFIG_FILES {
        let cfg_path = app_data.join(name);
        if cfg_path.exists() {
            if let Ok(content) = fs::read_to_string(&cfg_path) {
                let entry = format!("configs/{}", name);
                zip.start_file(&entry, options)
                    .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
                zip.write_all(content.as_bytes())
                    .map_err(KokoroError::from)?;
                config_count += 1;
            }
        }
    }

    zip.finish()
        .map_err(|e| KokoroError::Internal(format!("ZIP finish error: {}", e)))?;

    let size_bytes = fs::metadata(out_path).map(|m| m.len()).unwrap_or(0);
    stats.configs = config_count;

    Ok(ExportResult {
        path: out_path.to_string_lossy().to_string(),
        size_bytes,
        stats,
    })
}

#[tauri::command]
pub async fn preview_import(file_path: String) -> Result<ImportPreview, KokoroError> {
    let path = PathBuf::from(&file_path);
    let file = fs::File::open(&path).map_err(KokoroError::from)?;
    let mut archive = zip::ZipArchive::new(file)
        .map_err(|e| KokoroError::Internal(format!("Invalid ZIP archive: {}", e)))?;

    // Read manifest
    let manifest: BackupManifest = {
        let mut entry = archive.by_name("manifest.json").map_err(|_| {
            KokoroError::Validation("Missing manifest.json in backup file".to_string())
        })?;
        let mut buf = String::new();
        entry.read_to_string(&mut buf).map_err(KokoroError::from)?;
        serde_json::from_str(&buf)
            .map_err(|e| KokoroError::Internal(format!("Invalid manifest: {}", e)))?
    };

    let has_database = archive.by_name("kokoro.db").is_ok();

    // Collect config file names
    let mut config_files: Vec<String> = Vec::new();
    for i in 0..archive.len() {
        if let Ok(entry) = archive.by_index(i) {
            let name = entry.name().to_string();
            if name.starts_with("configs/") && name.len() > 8 {
                config_files.push(name.trim_start_matches("configs/").to_string());
            }
        }
    }
    let has_configs = !config_files.is_empty();

    // If DB present, extract to temp and count rows
    let stats = if has_database {
        let tmp_guard = create_scoped_temp_dir("kokoro_import_preview")?;
        let tmp_dir_path = tmp_guard.path();
        let tmp_db = tmp_dir_path.join("preview.db");

        {
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| KokoroError::Internal(format!("Failed to read DB from ZIP: {}", e)))?;
            let mut out = fs::File::create(&tmp_db).map_err(KokoroError::from)?;
            std::io::copy(&mut entry, &mut out).map_err(KokoroError::from)?;
        }

        gather_stats(&tmp_db).await
    } else {
        BackupStats {
            memories: 0,
            conversations: 0,
            messages: 0,
            configs: 0,
        }
    };

    Ok(ImportPreview {
        manifest,
        has_database,
        has_configs,
        config_files,
        stats,
    })
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    file_path: String,
    options: ImportOptions,
) -> Result<ImportResult, KokoroError> {
    let app_data = app_data_dir(&app)?;
    let staged_configs = if options.import_configs {
        stage_backup_configs(Path::new(&file_path))?
    } else {
        Vec::new()
    };

    // Phase 1: Extract everything from ZIP synchronously (ZipFile is !Send)
    let tmp_guard = create_scoped_temp_dir("kokoro_import")?;
    let tmp_dir = tmp_guard.path();

    // Validate every resource before touching live database rows or package paths.
    let inspection = inspect_backup_archive(Path::new(&file_path))?;
    let resource_staging = tmp_dir.join("character-resources");
    let staged_resources = if should_stage_character_resources(&options, &inspection) {
        stage_character_resources(Path::new(&file_path), &resource_staging)?
    } else {
        StagedCharacterResources::default()
    };
    let mut has_db = false;
    let extracted_configs: Vec<(String, String)> = staged_configs
        .into_iter()
        .filter(|(filename, _)| {
            options.conflict_strategy != ConflictStrategy::Skip || !app_data.join(filename).exists()
        })
        .collect();

    {
        let path = PathBuf::from(&file_path);
        let file = fs::File::open(&path).map_err(KokoroError::from)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|e| KokoroError::Internal(format!("Invalid ZIP archive: {}", e)))?;

        // Extract DB if requested — always to a temp file to avoid clobbering the live DB
        if options.import_database && archive.by_name("kokoro.db").is_ok() {
            has_db = true;
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| KokoroError::Internal(format!("Failed to read DB: {}", e)))?;
            let mut out = fs::File::create(tmp_dir.join("import.db")).map_err(KokoroError::from)?;
            std::io::copy(&mut entry, &mut out).map_err(KokoroError::from)?;
        }
    }
    // archive is dropped here — safe to .await below
    let mut promoted_resources = if has_db
        && (!staged_resources.packages.is_empty() || !staged_resources.instance_ids.is_empty())
    {
        promote_staged_resources(
            &resource_staging,
            &app_data.join("characters"),
            &staged_resources,
            options.conflict_strategy,
        )?
    } else {
        ResourcePromotionGuard::empty()
    };
    let prepared_characters = if has_db {
        let source_pool = open_readonly_pool(&tmp_dir.join("import.db")).await?;
        let catalog_root = app_data.join("characters");
        let local_resolver = LocalCatalogPackageResolver::new(catalog_root.clone());
        let official_resolver = OfficialRegistryPackageResolver::new(catalog_root);
        let references = sqlx::query(
            "SELECT DISTINCT template_id, template_version FROM characters \
             WHERE template_id IS NOT NULL AND template_version IS NOT NULL",
        )
        .fetch_all(&source_pool)
        .await?;
        for reference in references {
            let template_id: String = reference.try_get("template_id")?;
            let template_version: String = reference.try_get("template_version")?;
            if local_resolver
                .resolve_exact(&template_id, &template_version)
                .map_err(KokoroError::Validation)?
                .is_none()
            {
                let _ = official_resolver
                    .hydrate_exact(&template_id, &template_version)
                    .await
                    .map_err(KokoroError::Validation)?;
            }
        }
        let rows = prepare_character_rows(&source_pool, &official_resolver).await?;
        source_pool.close().await;
        rows
    } else {
        Vec::new()
    };

    // Phase 2: Async DB operations
    let mut result = ImportResult {
        imported_memories: 0,
        imported_conversations: 0,
        imported_configs: 0,
        imported_characters: 0,
        // Kept only for wire compatibility with old frontends; SQLite is authoritative.
        characters_json: None,
        debug_log: Vec::new(),
    };
    let mut config_replacement = replace_configs_atomically(&app_data, &extracted_configs)?;
    result.imported_configs = config_replacement.imported_count();

    if has_db {
        let import_pool = resolve_import_pool(&app, &app_data).await?;
        let tmp_db = tmp_dir.join("import.db");
        // 必须用同一个连接：ATTACH DATABASE 是连接级别的操作
        let mut conn = import_pool.acquire().await.map_err(|e| {
            KokoroError::Database(format!("Failed to acquire DB connection: {}", e))
        })?;

        let attach_path = tmp_db.to_string_lossy().replace('\\', "/");
        tracing::info!(target: "backup", "[Backup] Attaching import DB from: {}", attach_path);
        // 使用参数绑定防止 SQL 注入
        sqlx::query("ATTACH DATABASE ? AS import_db")
            .bind(&attach_path)
            .execute(&mut *conn)
            .await
            .map_err(|e| KokoroError::Database(format!("ATTACH failed: {}", e)))?;

        // 验证 ATTACH 成功，能读到数据
        let import_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM import_db.memories")
            .fetch_one(&mut *conn)
            .await
            .map_err(|e| {
                KokoroError::Database(format!("Failed to count import_db.memories: {}", e))
            })?;
        tracing::info!(target: "backup", "[Backup] import_db.memories count: {}", import_count);
        result
            .debug_log
            .push(format!("import_db.memories count: {}", import_count));

        let import_memory_columns: Vec<String> =
            sqlx::query("PRAGMA import_db.table_info(memories)")
                .fetch_all(&mut *conn)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|row| row.get::<String, _>("name"))
                .collect();
        let memory_column_defaults = [
            ("updated_at", "INTEGER NOT NULL DEFAULT 0"),
            ("character_id", "TEXT NOT NULL DEFAULT 'default'"),
            ("tier", "TEXT NOT NULL DEFAULT 'ephemeral'"),
            ("consolidated_from", "TEXT"),
            ("memory_type", "TEXT NOT NULL DEFAULT 'legacy_fact'"),
            ("entity_key", "TEXT"),
            ("status", "TEXT NOT NULL DEFAULT 'active'"),
            ("confidence", "REAL NOT NULL DEFAULT 0.6"),
            ("first_seen_at", "INTEGER NOT NULL DEFAULT 0"),
            ("last_seen_at", "INTEGER NOT NULL DEFAULT 0"),
            ("evidence_count", "INTEGER NOT NULL DEFAULT 1"),
            ("source_kind", "TEXT NOT NULL DEFAULT 'legacy'"),
            ("source_refs", "TEXT NOT NULL DEFAULT '[]'"),
            ("supersedes", "TEXT"),
            ("canonical_hash", "TEXT"),
            ("last_dreamed_at", "INTEGER"),
        ];
        for (column, definition) in memory_column_defaults {
            if !import_memory_columns
                .iter()
                .any(|existing| existing == column)
            {
                let sql =
                    format!("ALTER TABLE import_db.memories ADD COLUMN {column} {definition}");
                sqlx::query(&sql).execute(&mut *conn).await.map_err(|e| {
                    KokoroError::Database(format!(
                        "Failed to normalize import memory column {column}: {e}"
                    ))
                })?;
            }
        }
        sqlx::query(
            "UPDATE import_db.memories \
             SET first_seen_at = CASE WHEN first_seen_at = 0 THEN created_at ELSE first_seen_at END, \
                 last_seen_at = CASE \
                    WHEN last_seen_at = 0 AND updated_at > created_at THEN updated_at \
                    WHEN last_seen_at = 0 THEN created_at \
                    ELSE last_seen_at \
                 END, \
                 canonical_hash = CASE \
                    WHEN canonical_hash IS NULL OR canonical_hash = '' THEN lower(trim(content)) \
                    ELSE canonical_hash \
                 END",
        )
        .execute(&mut *conn)
        .await
        .ok();
        sqlx::query(
            "UPDATE import_db.memories \
             SET memory_type = CASE \
                    WHEN substr(content, 1, 6) = '[type:' AND instr(content, ']') > 0 THEN \
                        CASE \
                            WHEN instr(content, '|') > 0 AND instr(content, '|') < instr(content, ']') THEN substr(content, 7, instr(content, '|') - 7) \
                            ELSE substr(content, 7, instr(content, ']') - 7) \
                        END \
                    ELSE memory_type \
                 END, \
                 entity_key = CASE \
                    WHEN instr(content, '|key:') > 0 AND instr(content, ']') > instr(content, '|key:') THEN \
                        substr(content, instr(content, '|key:') + 5, instr(content, ']') - (instr(content, '|key:') + 5)) \
                    ELSE entity_key \
                 END \
             WHERE substr(content, 1, 6) = '[type:'",
        )
        .execute(&mut *conn)
        .await
        .ok();

        // 打印备份里实际的 character_id 分布
        let char_ids: Vec<String> =
            sqlx::query_scalar("SELECT DISTINCT character_id FROM import_db.memories")
                .fetch_all(&mut *conn)
                .await
                .unwrap_or_default();
        tracing::info!(target: "backup", "[Backup] import_db.memories character_ids: {:?}", char_ids);
        result
            .debug_log
            .push(format!("import_db character_ids: {:?}", char_ids));
        result.debug_log.push(format!(
            "target_character_id: {:?}",
            options.target_character_id
        ));

        let import_conversation_columns: Vec<String> =
            sqlx::query("PRAGMA import_db.table_info(conversations)")
                .fetch_all(&mut *conn)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|row| row.get::<String, _>("name"))
                .collect();
        let import_has_topic = import_conversation_columns.iter().any(|col| col == "topic");
        let import_has_pinned_state = import_conversation_columns
            .iter()
            .any(|col| col == "pinned_state");
        result.debug_log.push(format!(
            "import conversations columns: {:?}",
            import_conversation_columns
        ));

        let conversation_insert_sql = if import_has_topic && import_has_pinned_state {
            "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at)
             SELECT id, character_id, title, topic, pinned_state, created_at, updated_at FROM import_db.conversations"
        } else if import_has_topic {
            "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at)
             SELECT id, character_id, title, topic, '{}' as pinned_state, created_at, updated_at FROM import_db.conversations"
        } else {
            "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at)
             SELECT id, character_id, title, '' as topic, '{}' as pinned_state, created_at, updated_at FROM import_db.conversations"
        };
        let conversation_insert_skip_sql = if import_has_topic && import_has_pinned_state {
            "INSERT OR IGNORE INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at)
             SELECT id, character_id, title, topic, pinned_state, created_at, updated_at FROM import_db.conversations"
        } else if import_has_topic {
            "INSERT OR IGNORE INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at)
             SELECT id, character_id, title, topic, '{}' as pinned_state, created_at, updated_at FROM import_db.conversations"
        } else {
            "INSERT OR IGNORE INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at)
             SELECT id, character_id, title, '' as topic, '{}' as pinned_state, created_at, updated_at FROM import_db.conversations"
        };
        let memory_insert_sql = "INSERT INTO memories \
             (id, content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from, \
              memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
              source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at) \
             SELECT id, content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from, \
                    memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
                    source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at FROM import_db.memories";
        let memory_insert_skip_sql = "INSERT OR IGNORE INTO memories \
             (id, content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from, \
              memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
              source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at) \
             SELECT id, content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from, \
                    memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
                    source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at FROM import_db.memories";

        // Every live-table mutation below shares this connection transaction. DDL for
        // the FTS triggers is transactional in SQLite, so any error restores both data
        // and trigger state before the connection is released.
        let mut transaction = conn.begin().await?;

        if options.conflict_strategy == ConflictStrategy::Overwrite {
            // 先删除 FTS 触发器，避免批量操作时触发器访问损坏的 FTS 索引
            sqlx::query("DROP TRIGGER IF EXISTS memories_ai")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DROP TRIGGER IF EXISTS memories_ad")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("DROP TRIGGER IF EXISTS memories_au")
                .execute(&mut *transaction)
                .await?;

            sqlx::query("DELETE FROM conversation_messages")
                .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("DELETE conversation_messages failed: {}", e))
                })?;
            sqlx::query("DELETE FROM conversations")
                .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("DELETE conversations failed: {}", e))
                })?;
            sqlx::query("DELETE FROM memories")
                .execute(&mut *transaction)
                .await
                .map_err(|e| KokoroError::Database(format!("DELETE memories failed: {}", e)))?;

            let r = sqlx::query(memory_insert_sql)
                .execute(&mut *transaction)
                .await
                .map_err(|e| KokoroError::Database(format!("INSERT memories failed: {}", e)))?;
            result.imported_memories = r.rows_affected() as i64;
            tracing::info!(target: "backup", "[Backup] Inserted {} memories", result.imported_memories);
            result
                .debug_log
                .push(format!("inserted memories: {}", result.imported_memories));

            for table in [
                "memory_candidates",
                "memory_evidence",
                "memory_dream_jobs",
                "memory_dream_proposals",
                "memory_operations",
            ] {
                let import_has_table: Option<String> = sqlx::query_scalar(
                    "SELECT name FROM import_db.sqlite_master WHERE type='table' AND name = ?",
                )
                .bind(table)
                .fetch_optional(&mut *transaction)
                .await?;
                if import_has_table.is_some() {
                    sqlx::query(&format!("DELETE FROM {table}"))
                        .execute(&mut *transaction)
                        .await?;
                    sqlx::query(&format!(
                        "INSERT INTO {table} SELECT * FROM import_db.{table}"
                    ))
                    .execute(&mut *transaction)
                    .await?;
                }
            }

            let r = sqlx::query(conversation_insert_sql)
                .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("INSERT conversations failed: {}", e))
                })?;
            result.imported_conversations = r.rows_affected() as i64;
            result.debug_log.push(format!(
                "inserted conversations: {}",
                result.imported_conversations
            ));

            sqlx::query(
                "INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at)
                 SELECT id, conversation_id, role, content, metadata, created_at FROM import_db.conversation_messages",
            )
                .execute(&mut *transaction)
            .await
            .map_err(|e| {
                KokoroError::Database(format!("INSERT conversation_messages failed: {}", e))
            })?;

            // 重建 FTS 索引并恢复触发器
            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END").execute(&mut *transaction).await?;
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content); END").execute(&mut *transaction).await?;
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content); INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END").execute(&mut *transaction).await?;
        } else {
            // skip 模式：先重建 FTS 以防损坏
            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
                .execute(&mut *transaction)
                .await?;

            let r = sqlx::query(memory_insert_skip_sql)
                .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("INSERT OR IGNORE memories failed: {}", e))
                })?;
            result.imported_memories = r.rows_affected() as i64;
            tracing::info!(
                target: "backup",
                "[Backup] Inserted {} memories (skip mode)",
                result.imported_memories
            );
            result.debug_log.push(format!(
                "inserted memories (skip): {}",
                result.imported_memories
            ));

            for table in [
                "memory_candidates",
                "memory_evidence",
                "memory_dream_jobs",
                "memory_dream_proposals",
                "memory_operations",
            ] {
                let import_has_table: Option<String> = sqlx::query_scalar(
                    "SELECT name FROM import_db.sqlite_master WHERE type='table' AND name = ?",
                )
                .bind(table)
                .fetch_optional(&mut *transaction)
                .await?;
                if import_has_table.is_some() {
                    sqlx::query(&format!(
                        "INSERT OR IGNORE INTO {table} SELECT * FROM import_db.{table}"
                    ))
                    .execute(&mut *transaction)
                    .await?;
                }
            }

            let r = sqlx::query(conversation_insert_skip_sql)
                .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("INSERT OR IGNORE conversations failed: {}", e))
                })?;
            result.imported_conversations = r.rows_affected() as i64;
            result.debug_log.push(format!(
                "inserted conversations (skip): {}",
                result.imported_conversations
            ));

            sqlx::query("INSERT OR IGNORE INTO conversation_messages (id, conversation_id, role, content, metadata, created_at) SELECT id, conversation_id, role, content, metadata, created_at FROM import_db.conversation_messages")
                .execute(&mut *transaction).await
                .map_err(|e| KokoroError::Database(format!("INSERT OR IGNORE conversation_messages failed: {}", e)))?;

            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
                .execute(&mut *transaction)
                .await?;
        }

        // 如果指定了目标 character_id，把所有导入的记忆和对话重映射过去
        if let Some(ref target_id) = options.target_character_id {
            tracing::info!(target: "backup", "[Backup] Remapping character_id to '{}'", target_id);
            result
                .debug_log
                .push(format!("remapping all character_ids to: {}", target_id));
            let r = sqlx::query("UPDATE memories SET character_id = ? WHERE character_id != ?")
                .bind(target_id)
                .bind(target_id)
                .execute(&mut *transaction)
                .await?;
            result
                .debug_log
                .push(format!("memories remapped: {}", r.rows_affected()));
            sqlx::query("UPDATE conversations SET character_id = ? WHERE character_id != ?")
                .bind(target_id)
                .bind(target_id)
                .execute(&mut *transaction)
                .await?;
            for table in [
                "memory_candidates",
                "memory_evidence",
                "memory_dream_jobs",
                "memory_dream_proposals",
                "memory_operations",
            ] {
                sqlx::query(&format!(
                    "UPDATE {table} SET character_id = ? WHERE character_id != ?"
                ))
                .bind(target_id)
                .bind(target_id)
                .execute(&mut *transaction)
                .await?;
            }
        } else {
            result
                .debug_log
                .push("no target_character_id — remap skipped".to_string());
        }

        result.imported_characters = apply_character_rows(
            &mut transaction,
            prepared_characters,
            options.conflict_strategy.as_str(),
        )
        .await?;
        transaction.commit().await?;

        // Database rows now reference the imported files. Do not let a later
        // connection-level DETACH error remove those committed resources.
        promoted_resources.disarm();
        config_replacement.disarm();

        detach_import_database_best_effort(&mut conn).await;

        // Persist only after every live database table has committed.
        if let Some(ref target_id) = options.target_character_id {
            crate::ai::context::AIOrchestrator::persist_active_character_id(target_id);
            result
                .debug_log
                .push(format!("persisted active_character_id: {}", target_id));
        }

        drop(conn);
        // tmp_db 由 _tmp_guard 在函数结束时自动清理，无需手动删除
    }

    // Imports without a database have no committed rows to coordinate with.
    promoted_resources.disarm();
    config_replacement.disarm();

    // tmp_dir 由 _tmp_guard 自动清理

    tracing::info!(
        target: "backup",
        "[Backup] Imported: {} memories, {} conversations, {} configs",
        result.imported_memories, result.imported_conversations, result.imported_configs
    );

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn open_import_pool_without_orchestrator_creates_usable_db() {
        let tmp = tempfile::tempdir().expect("failed to create tempdir");
        let app_data = tmp.path().to_path_buf();

        let pool = open_import_pool_without_orchestrator(&app_data)
            .await
            .expect("fallback pool should open");

        let table_exists: Option<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='memories'",
        )
        .fetch_optional(&pool)
        .await
        .expect("query should succeed");

        assert_eq!(table_exists.as_deref(), Some("memories"));

        pool.close().await;
    }

    #[tokio::test]
    async fn late_character_failure_rolls_back_prior_destructive_import_mutations() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        for statement in [
            "CREATE TABLE memories (id INTEGER PRIMARY KEY)",
            "CREATE TABLE conversations (id TEXT PRIMARY KEY)",
            "CREATE TABLE conversation_messages (id INTEGER PRIMARY KEY)",
            "CREATE TABLE characters (id TEXT PRIMARY KEY, name TEXT NOT NULL CHECK(name != 'reject'), persona TEXT NOT NULL, user_nickname TEXT NOT NULL, source_format TEXT NOT NULL, created_at INTEGER NOT NULL, updated_at INTEGER NOT NULL, template_id TEXT, template_version TEXT, template_snapshot_json TEXT, description TEXT NOT NULL, avatar_path TEXT, greeting TEXT NOT NULL, greeting_consumed_at INTEGER, greeting_message_id INTEGER, example_dialogue TEXT NOT NULL, runtime_profile_json TEXT NOT NULL, user_modified_at INTEGER)",
            "INSERT INTO memories VALUES (1)",
            "INSERT INTO conversations VALUES ('old')",
            "INSERT INTO conversation_messages VALUES (1)",
            "INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at, description, greeting, example_dialogue, runtime_profile_json) VALUES ('old', 'Old', '', '', 'manual', 1, 1, '', '', '', '{}')",
        ] {
            sqlx::query(statement).execute(&pool).await.unwrap();
        }
        let mut transaction = pool.begin().await.unwrap();
        sqlx::query("DELETE FROM conversation_messages")
            .execute(&mut *transaction)
            .await
            .unwrap();
        sqlx::query("DELETE FROM conversations")
            .execute(&mut *transaction)
            .await
            .unwrap();
        sqlx::query("DELETE FROM memories")
            .execute(&mut *transaction)
            .await
            .unwrap();
        let failure = apply_character_rows(
            &mut transaction,
            vec![PreparedCharacterRow {
                id: "new".to_string(),
                name: "reject".to_string(),
                persona: String::new(),
                user_nickname: String::new(),
                source_format: "manual".to_string(),
                created_at: 2,
                updated_at: 2,
                template_id: None,
                template_version: None,
                template_snapshot_json: None,
                description: String::new(),
                avatar_path: None,
                greeting: String::new(),
                greeting_consumed_at: None,
                greeting_message_id: None,
                example_dialogue: String::new(),
                runtime_profile_json: "{}".to_string(),
                user_modified_at: None,
            }],
            "overwrite",
        )
        .await;
        assert!(failure.is_err());
        transaction.rollback().await.unwrap();

        for (table, expected) in [
            ("memories", 1_i64),
            ("conversations", 1_i64),
            ("conversation_messages", 1_i64),
            ("characters", 1_i64),
        ] {
            let count: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&pool)
                .await
                .unwrap();
            assert_eq!(count, expected, "{table} must roll back");
        }
    }

    #[test]
    fn overwrite_resource_guard_restores_old_avatar_until_disarmed() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let catalog = temp.path().join("app/characters");
        let live = temp
            .path()
            .join("app/character-instance-resources/instance/avatar.png");
        fs::create_dir_all(live.parent().unwrap()).unwrap();
        fs::write(&live, b"old").unwrap();
        let staged_avatar = staging.join(".instances/instance/avatar.png");
        fs::create_dir_all(staged_avatar.parent().unwrap()).unwrap();
        fs::write(&staged_avatar, b"new").unwrap();
        let staged = StagedCharacterResources {
            packages: Vec::new(),
            instance_ids: vec!["instance".to_string()],
        };

        let guard =
            promote_staged_resources(&staging, &catalog, &staged, ConflictStrategy::Overwrite)
                .unwrap();
        assert_eq!(fs::read(&live).unwrap(), b"new");
        drop(guard);
        assert_eq!(fs::read(&live).unwrap(), b"old");

        fs::create_dir_all(staged_avatar.parent().unwrap()).unwrap();
        fs::write(&staged_avatar, b"committed").unwrap();
        let mut guard =
            promote_staged_resources(&staging, &catalog, &staged, ConflictStrategy::Overwrite)
                .unwrap();
        guard.disarm();
        drop(guard);
        assert_eq!(fs::read(&live).unwrap(), b"committed");
    }

    #[test]
    fn skip_resource_promotion_keeps_existing_managed_avatar() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let catalog = temp.path().join("app/characters");
        let live = temp
            .path()
            .join("app/character-instance-resources/instance/avatar.png");
        fs::create_dir_all(live.parent().unwrap()).unwrap();
        fs::write(&live, b"old").unwrap();
        let staged_avatar = staging.join(".instances/instance/avatar.png");
        fs::create_dir_all(staged_avatar.parent().unwrap()).unwrap();
        fs::write(&staged_avatar, b"new").unwrap();
        let staged = StagedCharacterResources {
            packages: Vec::new(),
            instance_ids: vec!["instance".to_string()],
        };

        let mut guard =
            promote_staged_resources(&staging, &catalog, &staged, ConflictStrategy::Skip).unwrap();
        guard.disarm();

        assert_eq!(fs::read(&live).unwrap(), b"old");
    }

    #[test]
    fn config_replacement_rolls_back_until_database_commit_disarms_it() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("llm_config.json");
        fs::write(&target, r#"{"old":true}"#).unwrap();
        let configs = vec![("llm_config.json".to_string(), r#"{"new":true}"#.to_string())];

        let guard = replace_configs_atomically(temp.path(), &configs).unwrap();
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"new":true}"#);
        drop(guard);
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"old":true}"#);

        let mut guard = replace_configs_atomically(temp.path(), &configs).unwrap();
        guard.disarm();
        drop(guard);
        assert_eq!(fs::read_to_string(&target).unwrap(), r#"{"new":true}"#);
    }

    #[test]
    fn import_temp_directories_are_uuid_scoped_and_cleanup_on_drop() {
        let first = create_scoped_temp_dir("kokoro_import").unwrap();
        let second = create_scoped_temp_dir("kokoro_import").unwrap();
        let first_path = first.path().to_path_buf();
        let second_path = second.path().to_path_buf();
        assert_ne!(first_path, second_path);
        assert!(first_path.is_dir());
        assert!(second_path.is_dir());
        drop(first);
        drop(second);
        assert!(!first_path.exists());
        assert!(!second_path.exists());
    }

    #[test]
    fn resources_are_staged_only_for_database_imports() {
        let inspection = BackupArchiveInspection {
            has_character_resources: true,
            includes_provider_credentials: false,
        };
        let mut options = ImportOptions {
            import_database: false,
            import_configs: true,
            conflict_strategy: ConflictStrategy::Overwrite,
            target_character_id: None,
        };
        assert!(!should_stage_character_resources(&options, &inspection));
        options.import_database = true;
        assert!(should_stage_character_resources(&options, &inspection));
    }

    #[tokio::test]
    async fn detach_failure_after_commit_is_best_effort() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();

        assert!(!detach_import_database_best_effort(&mut connection).await);
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT 1")
                .fetch_one(&mut *connection)
                .await
                .unwrap(),
            1
        );
    }
}
