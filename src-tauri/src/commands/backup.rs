// pattern: Imperative Shell

use crate::ai::context::{
    acquire_database_operation_read_guard, acquire_database_operation_write_guard, AIOrchestrator,
};
use crate::characters::activation::CommittedCharacterRuntime;
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
use crate::registry::manifest::{RegistryEntry, RegistryIndex, OFFICIAL_REGISTRY_URL};
use semver::Version;
use serde::{Deserialize, Serialize};
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{Acquire, Row, SqliteConnection, SqlitePool};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Cursor, Read, Seek, Write};
use std::path::{Component, Path, PathBuf};
use std::str::FromStr;
use tauri::AppHandle;
use tauri::Manager;
use uuid::Uuid;
use zip::write::SimpleFileOptions;

#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(windows)]
use std::os::windows::fs::{MetadataExt, OpenOptionsExt};

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
    "context_settings.json",
    "current_conversation_id.json",
    "user_profile.json",
];
/// Config entries produced by removed subsystems. They remain importable as
/// no-ops so old backups can still be opened without restoring dead state.
const LEGACY_IGNORED_CONFIG_FILES: &[&str] = &["emotion_state.json"];

/// Tables whose rows carry an integer reference into `memories.id`. A restore
/// that replaces `memories` must clear these even when the backup predates them,
/// because the same integer id now belongs to a different imported memory.
const MEMORY_REFERENCING_TABLES: &[&str] = &[
    "memory_candidates",
    "memory_evidence",
    "memory_dream_proposals",
    "memory_operations",
];

pub(crate) const MAX_BACKUP_RESOURCE_PACKAGES: usize = 64;
pub(crate) const MAX_BACKUP_RESOURCE_FILES: usize = 2_048;
pub(crate) const MAX_BACKUP_RESOURCE_BYTES: u64 = 512 * 1024 * 1024;
/// Bound decompressed database payloads before writing them to a temporary file.
pub(crate) const MAX_BACKUP_DATABASE_BYTES: u64 = 512 * 1024 * 1024;
/// Configuration files are small JSON documents; never inflate an archive entry
/// without a strict per-file bound.
pub(crate) const MAX_BACKUP_CONFIG_BYTES: u64 = 4 * 1024 * 1024;
const MAX_BACKUP_MANIFEST_BYTES: u64 = 64 * 1024;
/// Current archive format. Manual and automatic exports share one value: the
/// manifest version describes the archive layout, not the export entry point.
pub(crate) const BACKUP_FORMAT_VERSION: &str = "2";

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
        validate_catalog_segment(template_id, "template id")?;
        validate_catalog_segment(template_version, "template version")?;
        let package_dir = self.root.join(template_id).join(template_version);
        let package_parent = package_dir
            .parent()
            .ok_or_else(|| "character package has no parent".to_string())?;
        ensure_non_redirected_parent_chain(&self.root, package_parent)
            .map_err(|error| error.to_string())?;
        reject_case_folded_sibling(package_parent, "template id")
            .map_err(|error| error.to_string())?;
        reject_case_folded_sibling(&package_dir, "template version")
            .map_err(|error| error.to_string())?;
        let package_metadata = match fs::symlink_metadata(&package_dir) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.to_string()),
        };
        if is_filesystem_redirect(&package_metadata) || !package_metadata.is_dir() {
            return Err("character package is not a regular directory".to_string());
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
        ensure_non_redirected_parent_chain(app_data, directory)
            .map_err(|error| error.to_string())?;
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

fn find_official_character_package<'a>(
    index: &'a RegistryIndex,
    template_id: &str,
    template_version: &str,
) -> Option<&'a RegistryEntry> {
    if validate_catalog_segment(template_id, "template id").is_err()
        || validate_catalog_segment(template_version, "template version").is_err()
    {
        return None;
    }
    index.entries.iter().find(|entry| {
        entry.content_type == "character"
            && entry.id == template_id
            && entry.version == template_version
    })
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
        let Some(entry) = find_official_character_package(&index, template_id, template_version)
        else {
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

#[derive(Clone, Debug, Default, Serialize)]
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

/// One character instance stored inside a backup.
///
/// Instance ids are generated per machine, so a backup taken elsewhere never
/// reuses a local id. The preview exposes them so the user can decide, per
/// character, whether to import it as a new instance or merge its data into an
/// existing one.
#[derive(Clone, Debug, Serialize)]
pub struct BackupCharacterSummary {
    pub id: String,
    pub name: String,
    pub memory_count: i64,
    pub conversation_count: i64,
}

#[derive(Debug, Serialize)]
pub struct ImportPreview {
    pub manifest: BackupManifest,
    pub has_database: bool,
    pub has_configs: bool,
    pub config_files: Vec<String>,
    pub stats: BackupStats,
    /// Empty when the backup predates the SQLite character table.
    pub characters: Vec<BackupCharacterSummary>,
}

/// Routes the rows of one imported character to a local character instance.
#[derive(Clone, Debug, Deserialize)]
pub struct CharacterMerge {
    /// Character id as stored inside the backup.
    pub imported_id: String,
    /// Existing local character that receives the imported rows.
    pub target_id: String,
}

#[derive(Debug, Deserialize)]
pub struct ImportOptions {
    pub import_database: bool,
    pub import_configs: bool,
    pub conflict_strategy: ConflictStrategy,
    /// Characters to merge into an existing local instance. Every imported
    /// character that is neither listed here nor in `ignored_characters` is
    /// imported as a new instance.
    #[serde(default)]
    pub character_merges: Vec<CharacterMerge>,
    /// Characters whose rows must not be restored at all.
    #[serde(default)]
    pub ignored_characters: Vec<String>,
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
    /// Imported characters whose rows were routed into an existing local instance.
    pub merged_characters: i64,
    /// Imported characters whose rows were left out of the restore.
    pub ignored_characters: i64,
    pub characters_json: Option<String>,
    /// Memories the skip strategy dropped because the imported row violates a
    /// local constraint even though its id was free.
    pub skipped_memories: i64,
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

/// Read at most `max_bytes + 1` decompressed bytes from an archive entry. The
/// extra byte lets us distinguish an exact-limit payload from an oversized one
/// without trusting ZIP header sizes alone.
pub(crate) fn read_limited_bytes<R: Read>(
    input: R,
    max_bytes: u64,
    label: &str,
) -> Result<Vec<u8>, KokoroError> {
    let read_limit = max_bytes
        .checked_add(1)
        .ok_or_else(|| KokoroError::Validation(format!("{label} limit overflow")))?;
    let mut bytes = Vec::new();
    input
        .take(read_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| KokoroError::Validation(format!("failed to read {label}: {error}")))?;
    if bytes.len() as u64 > max_bytes {
        return Err(KokoroError::Validation(format!(
            "{label} exceeds decompressed size limit of {max_bytes} bytes"
        )));
    }
    Ok(bytes)
}

/// Exports must never overwrite the database, configuration, or any managed
/// character resource. Canonicalizing the existing parent also catches `..`,
/// case aliases, and Windows junction/reparse-point aliases before opening the
/// output. `ensure_non_redirected_parent_chain` remains the final Windows
/// junction protection when the file is created.
pub(crate) fn validate_export_target(app_data: &Path, out_path: &Path) -> Result<(), KokoroError> {
    let canonical_root = fs::canonicalize(app_data).map_err(|error| {
        KokoroError::Validation(format!(
            "managed app data root is not canonicalizable: {} ({error})",
            app_data.display()
        ))
    })?;
    let parent = out_path
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical_parent = fs::canonicalize(parent).map_err(|error| {
        KokoroError::Validation(format!(
            "managed backup export parent is not canonicalizable: {} ({error})",
            parent.display()
        ))
    })?;
    let file_name = out_path.file_name().ok_or_else(|| {
        KokoroError::Validation("backup export target has no filename".to_string())
    })?;
    let canonical_target = canonical_parent.join(file_name);
    if path_is_same_or_descendant(&canonical_target, &canonical_root) {
        return Err(KokoroError::Validation(format!(
            "backup export target is inside managed app data: {}",
            out_path.display()
        )));
    }
    Ok(())
}

fn path_is_same_or_descendant(path: &Path, root: &Path) -> bool {
    #[cfg(windows)]
    {
        let path = path.to_string_lossy().to_ascii_lowercase();
        let root = root.to_string_lossy().to_ascii_lowercase();
        path == root
            || path
                .strip_prefix(&root)
                .is_some_and(|suffix| suffix.starts_with('\\') || suffix.starts_with('/'))
    }
    #[cfg(not(windows))]
    {
        path == root || path.starts_with(root)
    }
}

fn open_regular_non_redirected_file(path: &Path, label: &str) -> Result<Option<File>, KokoroError> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(KokoroError::from(error)),
    };
    if is_filesystem_redirect(&metadata) || !metadata.is_file() {
        return Err(KokoroError::Validation(format!(
            "{label} is not a regular non-symlink file: {}",
            path.display()
        )));
    }

    let mut options = OpenOptions::new();
    options.read(true);
    configure_no_follow(&mut options);
    let file = options.open(path).map_err(|error| {
        KokoroError::Io(format!(
            "failed to open {label} {}: {error}",
            path.display()
        ))
    })?;
    let opened_metadata = file.metadata().map_err(KokoroError::from)?;
    if is_filesystem_redirect(&opened_metadata) || !opened_metadata.is_file() {
        return Err(KokoroError::Validation(format!(
            "{label} changed to a redirected or non-file path: {}",
            path.display()
        )));
    }
    Ok(Some(file))
}

struct AtomicExportGuard {
    temporary: PathBuf,
    committed: bool,
}

impl AtomicExportGuard {
    fn new(temporary: PathBuf) -> Self {
        Self {
            temporary,
            committed: false,
        }
    }

    fn disarm(&mut self) {
        self.committed = true;
    }
}

impl Drop for AtomicExportGuard {
    fn drop(&mut self) {
        if !self.committed {
            let _ = fs::remove_file(&self.temporary);
        }
    }
}

fn create_atomic_export_file(path: &Path) -> Result<(AtomicExportGuard, File), KokoroError> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        ensure_non_redirected_parent_chain(parent, parent)?;
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let name = path.file_name().ok_or_else(|| {
        KokoroError::Validation("backup export target has no filename".to_string())
    })?;
    let temporary = parent.join(format!(
        ".{}.{}.tmp",
        name.to_string_lossy(),
        Uuid::new_v4()
    ));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    configure_no_follow(&mut options);
    let file = options.open(&temporary).map_err(|error| {
        KokoroError::Io(format!(
            "failed to create temporary backup export {}: {error}",
            temporary.display()
        ))
    })?;
    let metadata = file.metadata().map_err(KokoroError::from)?;
    if is_filesystem_redirect(&metadata) || !metadata.is_file() {
        let _ = fs::remove_file(&temporary);
        return Err(KokoroError::Validation(format!(
            "temporary backup export is not a regular file: {}",
            temporary.display()
        )));
    }
    Ok((AtomicExportGuard::new(temporary), file))
}

fn commit_atomic_export(
    guard: &mut AtomicExportGuard,
    file: File,
    target: &Path,
) -> Result<(), KokoroError> {
    file.sync_all().map_err(|error| {
        KokoroError::Io(format!(
            "failed to sync temporary backup export {}: {error}",
            guard.temporary.display()
        ))
    })?;
    crate::config::atomic_replace_file(&guard.temporary, target).map_err(|error| {
        KokoroError::Io(format!(
            "failed to atomically replace backup export {}: {error}",
            target.display()
        ))
    })?;
    guard.disarm();
    Ok(())
}

/// A verified, self-consistent copy of the live database taken with `VACUUM INTO`.
struct DatabaseSnapshot {
    bytes: Vec<u8>,
    /// Counted from the snapshot itself, so the numbers the user sees always
    /// describe the bytes that were archived.
    stats: BackupStats,
}

async fn create_consistent_database_snapshot(
    source: &Path,
) -> Result<Option<DatabaseSnapshot>, KokoroError> {
    if open_regular_non_redirected_file(source, "database")?.is_none() {
        return Ok(None);
    }

    let parent = source.parent().unwrap_or_else(|| Path::new("."));
    ensure_non_redirected_parent_chain(parent, parent)?;
    let file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("kokoro.db");
    let snapshot = parent.join(format!(".{file_name}.{}.snapshot", Uuid::new_v4()));

    let result = async {
        let url = format!("sqlite://{}", source.to_string_lossy().replace('\\', "/"));
        let options = SqliteConnectOptions::from_str(&url)
            .map_err(|error| KokoroError::Internal(format!("invalid database path: {error}")))?;
        let pool = SqlitePool::connect_with(options)
            .await
            .map_err(|error| KokoroError::Database(format!("failed to open database: {error}")))?;
        let snapshot_sql = snapshot
            .to_string_lossy()
            .replace('\\', "/")
            .replace('\'', "''");
        sqlx::query(&format!("VACUUM INTO '{snapshot_sql}'"))
            .execute(&pool)
            .await
            .map_err(|error| {
                KokoroError::Database(format!("failed to create database snapshot: {error}"))
            })?;
        pool.close().await;

        let mut snapshot_file =
            open_regular_non_redirected_file(&snapshot, "database snapshot")?
                .ok_or_else(|| KokoroError::Io("database snapshot was not created".to_string()))?;
        let bytes = read_limited_bytes(
            &mut snapshot_file,
            MAX_BACKUP_DATABASE_BYTES,
            "database snapshot",
        )?;
        let validation_pool = open_readonly_pool(&snapshot).await?;
        let integrity: String = sqlx::query_scalar("PRAGMA integrity_check")
            .fetch_one(&validation_pool)
            .await
            .map_err(|error| {
                KokoroError::Database(format!("failed to validate database snapshot: {error}"))
            })?;
        let stats = BackupStats {
            memories: count_rows(&validation_pool, CountTable::Memories).await,
            conversations: count_rows(&validation_pool, CountTable::Conversations).await,
            messages: count_rows(&validation_pool, CountTable::ConversationMessages).await,
            configs: 0,
        };
        validation_pool.close().await;
        if integrity != "ok" {
            return Err(KokoroError::Database(format!(
                "database snapshot integrity check failed: {integrity}"
            )));
        }
        Ok(DatabaseSnapshot { bytes, stats })
    }
    .await;

    let _ = fs::remove_file(&snapshot);
    let _ = fs::remove_file(snapshot.with_extension("snapshot-wal"));
    let _ = fs::remove_file(snapshot.with_extension("snapshot-shm"));
    result.map(Some)
}

fn configure_no_follow(options: &mut OpenOptions) {
    #[cfg(unix)]
    {
        #[cfg(target_os = "linux")]
        const NO_FOLLOW: i32 = 0x20000;
        #[cfg(target_os = "macos")]
        const NO_FOLLOW: i32 = 0x100;
        #[cfg(not(any(target_os = "linux", target_os = "macos")))]
        const NO_FOLLOW: i32 = 0;
        options.custom_flags(NO_FOLLOW);
    }
    #[cfg(windows)]
    {
        const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x00200000;
        options.custom_flags(FILE_FLAG_OPEN_REPARSE_POINT);
    }
}

fn is_filesystem_redirect(metadata: &fs::Metadata) -> bool {
    #[cfg(windows)]
    {
        const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
        metadata.file_type().is_symlink()
            || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
    }
    #[cfg(not(windows))]
    {
        metadata.file_type().is_symlink()
    }
}

fn ensure_non_redirected_parent_chain(
    root: &Path,
    target_parent: &Path,
) -> Result<(), KokoroError> {
    let relative = target_parent.strip_prefix(root).map_err(|_| {
        KokoroError::Validation(format!(
            "restore target escapes managed root: {}",
            target_parent.display()
        ))
    })?;
    if relative
        .components()
        .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(KokoroError::Validation(format!(
            "restore target contains unsafe parent path: {}",
            target_parent.display()
        )));
    }

    // Walk to the nearest existing ancestor first, because create_dir_all can
    // otherwise follow a redirected ancestor while creating a missing root.
    let mut existing = target_parent.to_path_buf();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(metadata) => {
                if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                    return Err(KokoroError::Validation(format!(
                        "restore parent is not a regular directory: {}",
                        existing.display()
                    )));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or_else(|| {
                        KokoroError::Validation("restore target has no existing parent".to_string())
                    })?
                    .to_path_buf();
            }
            Err(error) => return Err(KokoroError::from(error)),
        }
    }

    // Validate every existing ancestor, including the managed root's parents,
    // before any rename or directory creation is attempted.
    let mut ancestor = existing.clone();
    loop {
        let metadata = fs::symlink_metadata(&ancestor).map_err(KokoroError::from)?;
        if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
            return Err(KokoroError::Validation(format!(
                "restore parent is not a regular directory: {}",
                ancestor.display()
            )));
        }
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if parent == ancestor {
            break;
        }
        ancestor = parent.to_path_buf();
    }

    // Validate each already-existing component below the nearest ancestor.
    let mut current = existing;
    let remaining = target_parent.strip_prefix(&current).map_err(|_| {
        KokoroError::Validation(format!(
            "restore target escapes existing parent: {}",
            target_parent.display()
        ))
    })?;
    for component in remaining.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                    return Err(KokoroError::Validation(format!(
                        "restore parent is not a regular directory: {}",
                        current.display()
                    )));
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => break,
            Err(error) => return Err(KokoroError::from(error)),
        }
    }
    Ok(())
}

fn reject_case_folded_sibling(path: &Path, label: &str) -> Result<(), std::io::Error> {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("invalid {label} path"),
        ));
    };
    let Some(parent) = path.parent() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("{label} has no parent"),
        ));
    };
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let existing = entry?.file_name();
        let existing = existing.to_string_lossy();
        if existing != name && existing.eq_ignore_ascii_case(name) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                format!("case-insensitive {label} collision between `{name}` and `{existing}`"),
            ));
        }
    }
    Ok(())
}

fn create_non_redirected_directory_chain(path: &Path) -> Result<(), KokoroError> {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                    return Err(KokoroError::Validation(format!(
                        "restore parent is not a regular directory: {}",
                        current.display()
                    )));
                }
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                missing.push(current.clone());
                current = current
                    .parent()
                    .ok_or_else(|| {
                        KokoroError::Validation("restore path has no parent".to_string())
                    })?
                    .to_path_buf();
            }
            Err(error) => return Err(KokoroError::from(error)),
        }
    }
    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(KokoroError::from(error)),
        }
        let metadata = fs::symlink_metadata(&directory).map_err(KokoroError::from)?;
        if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
            return Err(KokoroError::Validation(format!(
                "restore parent is not a regular directory: {}",
                directory.display()
            )));
        }
    }
    Ok(())
}

fn remove_non_redirected_directory(path: &Path) -> Result<(), std::io::Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        ensure_non_redirected_parent_chain(parent, parent)
            .map_err(|error| std::io::Error::other(error.to_string()))?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!(
                "refusing to remove redirected directory: {}",
                path.display()
            ),
        ));
    }
    fs::remove_dir_all(path)
}

fn validate_catalog_segment(value: &str, field: &str) -> Result<(), String> {
    let valid = if field == "template version" {
        let path = Path::new(value);
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && path.components().count() == 1
            && path.file_name().and_then(|name| name.to_str()) == Some(value)
            && Version::parse(value).is_ok()
    } else {
        !value.is_empty()
            && value.len() <= 128
            && value != "."
            && value != ".."
            && value
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
    };
    if valid {
        Ok(())
    } else {
        Err(format!("invalid {field} path segment"))
    }
}

fn archive_path_key(name: &str, is_directory: bool) -> Result<String, KokoroError> {
    let normalized = canonical_archive_path(name, is_directory)?;
    Ok(normalized.to_lowercase())
}

fn canonical_archive_path(name: &str, is_directory: bool) -> Result<String, KokoroError> {
    if name.is_empty() || name.contains('\\') {
        return Err(KokoroError::Validation(format!(
            "archive path uses a non-canonical separator: {name}"
        )));
    }
    let without_directory_suffix = if is_directory {
        name.strip_suffix('/').unwrap_or(name)
    } else {
        name
    };
    if without_directory_suffix.is_empty()
        || without_directory_suffix.starts_with('/')
        || without_directory_suffix.ends_with('/')
    {
        return Err(KokoroError::Validation(format!(
            "archive path contains repeated or absolute separators: {name}"
        )));
    }
    let mut components = Vec::new();
    for component in without_directory_suffix.split('/') {
        if component.is_empty() || matches!(component, "." | "..") {
            return Err(KokoroError::Validation(format!(
                "archive path contains a non-canonical separator or component: {name}"
            )));
        }
        components.push(component);
    }
    let canonical = components.join("/");
    Ok(if is_directory {
        format!("{canonical}/")
    } else {
        canonical
    })
}

fn validate_archive_path_uniqueness<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<(), KokoroError> {
    let mut seen = HashSet::new();
    for index in 0..archive.len() {
        let entry = archive
            .by_index(index)
            .map_err(|error| KokoroError::Validation(format!("invalid backup entry: {error}")))?;
        let path_key = archive_path_key(entry.name(), entry.is_dir())?;
        if !seen.insert(path_key) {
            return Err(KokoroError::Validation(format!(
                "duplicate backup archive path: {}",
                entry.name()
            )));
        }
    }
    Ok(())
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
        let _ = remove_non_redirected_directory(&self.0);
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
    validate_archive_path_uniqueness(&mut archive)?;
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
        if !seen.insert(filename.to_string()) {
            return Err(KokoroError::Validation(format!(
                "duplicate config filename: {filename}"
            )));
        }
        if LEGACY_IGNORED_CONFIG_FILES.contains(&filename) {
            continue;
        }
        if !allowed.contains(filename) {
            return Err(KokoroError::Validation(format!(
                "unknown config filename: {filename}"
            )));
        }
        let content = String::from_utf8(read_limited_bytes(
            &mut entry,
            MAX_BACKUP_CONFIG_BYTES,
            &format!("config {filename}"),
        )?)
        .map_err(|error| {
            KokoroError::Validation(format!("invalid UTF-8 config {filename}: {error}"))
        })?;
        serde_json::from_str::<serde_json::Value>(&content).map_err(|error| {
            KokoroError::Validation(format!("invalid JSON config {filename}: {error}"))
        })?;
        staged.push((filename.to_string(), content));
    }
    Ok(staged)
}

fn should_import_config(filename: &str, import_database: bool, has_database: bool) -> bool {
    filename != "current_conversation_id.json" || (import_database && has_database)
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
    ensure_non_redirected_parent_chain(app_data, app_data)?;
    create_non_redirected_directory_chain(app_data)?;
    for (filename, _) in configs {
        let target = app_data.join(filename);
        if let Ok(metadata) = fs::symlink_metadata(&target) {
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
        let prepared_content = match prepare_backup_config_for_install(app_data, filename, content)
        {
            Ok(content) => content,
            Err(error) => {
                for (_, staged, _, _, _) in &plans {
                    let _ = fs::remove_file(staged);
                }
                return Err(error);
            }
        };
        if let Err(error) = fs::write(&temporary, prepared_content) {
            for (_, staged, _, _, _) in &plans {
                let _ = fs::remove_file(staged);
            }
            return Err(error.into());
        }
        plans.push((target, temporary, backup, false, false));
    }

    let result = (|| -> Result<(), std::io::Error> {
        for (target, temporary, backup, had_original, was_installed) in &mut plans {
            ensure_non_redirected_parent_chain(app_data, app_data)
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            if let Ok(metadata) = fs::symlink_metadata(&*target) {
                if metadata.file_type().is_symlink() || !metadata.file_type().is_file() {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        format!("config target is not a regular file: {}", target.display()),
                    ));
                }
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
            let _ = remove_non_redirected_directory(backup);
        }
        self.is_armed = false;
    }
}

impl Drop for ResourcePromotionGuard {
    fn drop(&mut self) {
        if self.is_armed {
            for target in self.created_targets.iter().rev() {
                let _ = remove_non_redirected_directory(target);
            }
            for (target, backup) in self.replaced_targets.iter().rev() {
                let _ = remove_non_redirected_directory(target);
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
        let target_parent = target.parent().ok_or_else(|| {
            KokoroError::Validation("character package target has no parent".to_string())
        })?;
        ensure_non_redirected_parent_chain(catalog_root, target_parent)?;
        reject_case_folded_sibling(target_parent, "template id").map_err(KokoroError::from)?;
        reject_case_folded_sibling(&target, "template version").map_err(KokoroError::from)?;
        ensure_staged_directory(&source)?;
        if let Some(metadata) = match fs::symlink_metadata(&target) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(KokoroError::from(error)),
        } {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
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
            ensure_non_redirected_parent_chain(catalog_root, target_parent)?;
            let current = fs::symlink_metadata(&target).map_err(KokoroError::from)?;
            if is_filesystem_redirect(&current) || !current.is_dir() {
                return Err(KokoroError::Validation(format!(
                    "installed character package target {id}@{version} changed during restore"
                )));
            }
            fs::rename(&target, &backup).map_err(KokoroError::from)?;
            ensure_non_redirected_parent_chain(catalog_root, target_parent)?;
            ensure_staged_directory(&source)?;
            if let Err(error) = fs::rename(&source, &target) {
                let _ = fs::rename(&backup, &target);
                return Err(error.into());
            }
            guard.replaced_targets.push((target, backup));
            continue;
        }
        create_non_redirected_directory_chain(target_parent)?;
        ensure_non_redirected_parent_chain(catalog_root, target_parent)?;
        ensure_staged_directory(&source)?;
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
        let target_parent = target.parent().ok_or_else(|| {
            KokoroError::Validation("managed avatar target has no parent".to_string())
        })?;
        ensure_non_redirected_parent_chain(app_data, target_parent)?;
        reject_case_folded_sibling(target_parent, "managed character instance id")
            .map_err(KokoroError::from)?;
        reject_case_folded_sibling(&target, "managed character instance id")
            .map_err(KokoroError::from)?;
        ensure_staged_directory(&source)?;
        if let Some(metadata) = match fs::symlink_metadata(&target) {
            Ok(metadata) => Some(metadata),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
            Err(error) => return Err(KokoroError::from(error)),
        } {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err(KokoroError::Validation(format!(
                    "installed managed avatar target for {instance_id} is unsafe"
                )));
            }
            if conflict_strategy == ConflictStrategy::Skip {
                continue;
            }
            let backup =
                target.with_file_name(format!(".{}.import-backup-{}", instance_id, Uuid::new_v4()));
            ensure_non_redirected_parent_chain(app_data, target_parent)?;
            let current = fs::symlink_metadata(&target).map_err(KokoroError::from)?;
            if is_filesystem_redirect(&current) || !current.is_dir() {
                return Err(KokoroError::Validation(format!(
                    "installed managed avatar target for {instance_id} changed during restore"
                )));
            }
            fs::rename(&target, &backup).map_err(KokoroError::from)?;
            ensure_non_redirected_parent_chain(app_data, target_parent)?;
            ensure_staged_directory(&source)?;
            if let Err(error) = fs::rename(&source, &target) {
                let _ = fs::rename(&backup, &target);
                return Err(error.into());
            }
            guard.replaced_targets.push((target, backup));
            continue;
        }
        create_non_redirected_directory_chain(target_parent)?;
        ensure_non_redirected_parent_chain(app_data, target_parent)?;
        ensure_staged_directory(&source)?;
        fs::rename(source, &target).map_err(KokoroError::from)?;
        guard.created_targets.push(target);
    }
    Ok(guard)
}

fn ensure_staged_directory(path: &Path) -> Result<(), KokoroError> {
    let metadata = fs::symlink_metadata(path).map_err(KokoroError::from)?;
    if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
        return Err(KokoroError::Validation(format!(
            "staged restore source is not a regular directory: {}",
            path.display()
        )));
    }
    Ok(())
}

fn package_directory_entries(root: &Path) -> Result<Vec<PackageContentEntry>, String> {
    let root_metadata = fs::symlink_metadata(root).map_err(|error| error.to_string())?;
    if is_filesystem_redirect(&root_metadata) || !root_metadata.is_dir() {
        return Err(format!(
            "character package root is not a regular directory: {}",
            root.display()
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(&directory).map_err(|error| error.to_string())? {
            let entry = entry.map_err(|error| error.to_string())?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path).map_err(|error| error.to_string())?;
            let file_type = metadata.file_type();
            if is_filesystem_redirect(&metadata) {
                return Err(format!(
                    "character package contains redirected entry: {}",
                    path.display()
                ));
            }
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
    validate_archive_path_uniqueness(&mut archive)?;
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
    ensure_non_redirected_parent_chain(staging_root, staging_root)?;
    match fs::symlink_metadata(staging_root) {
        Ok(metadata) => {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err(KokoroError::Validation(format!(
                    "resource staging root is not a regular directory: {}",
                    staging_root.display()
                )));
            }
            remove_non_redirected_directory(staging_root).map_err(KokoroError::from)?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(KokoroError::from(error)),
    }
    create_non_redirected_directory_chain(staging_root)?;
    let result = (|| -> Result<StagedCharacterResources, KokoroError> {
        let file = fs::File::open(backup_path).map_err(KokoroError::from)?;
        let mut archive = zip::ZipArchive::new(file)
            .map_err(|error| KokoroError::Validation(format!("invalid backup archive: {error}")))?;
        validate_archive_path_uniqueness(&mut archive)?;
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
            validate_catalog_segment(components[1], "template id")
                .map_err(KokoroError::Validation)?;
            validate_catalog_segment(components[2], "template version")
                .map_err(KokoroError::Validation)?;
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
        let _ = remove_non_redirected_directory(staging_root);
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

pub(crate) async fn load_template_references(
    source: &SqlitePool,
) -> Result<Vec<(String, String)>, KokoroError> {
    let columns: Vec<String> = sqlx::query("PRAGMA table_info(characters)")
        .fetch_all(source)
        .await?
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect();
    if !columns.iter().any(|column| column == "template_id")
        || !columns.iter().any(|column| column == "template_version")
    {
        return Ok(Vec::new());
    }

    let rows = sqlx::query(
        "SELECT DISTINCT template_id, template_version FROM characters \
         WHERE template_id IS NOT NULL AND template_version IS NOT NULL",
    )
    .fetch_all(source)
    .await?;
    rows.into_iter()
        .map(|row| {
            Ok((
                row.try_get::<String, _>("template_id")?,
                row.try_get::<String, _>("template_version")?,
            ))
        })
        .collect()
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
        let insert_sql = if conflict_strategy == "overwrite" {
            "INSERT INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at, template_id, template_version, template_snapshot_json, description, avatar_path, greeting, greeting_consumed_at, greeting_message_id, example_dialogue, runtime_profile_json, user_modified_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        } else {
            "INSERT OR IGNORE INTO characters (id, name, persona, user_nickname, source_format, created_at, updated_at, template_id, template_version, template_snapshot_json, description, avatar_path, greeting, greeting_consumed_at, greeting_message_id, example_dialogue, runtime_profile_json, user_modified_at) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"
        };
        let result = sqlx::query(insert_sql)
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

/// Character-scoped tables rewritten when an imported character is merged into
/// an existing local one. `conversation_messages` is reached through its
/// conversation, which is character-scoped.
const IMPORT_CHARACTER_SCOPED_TABLES: &[&str] = &[
    "memories",
    "conversations",
    "memory_candidates",
    "memory_evidence",
    "memory_dream_jobs",
    "memory_dream_proposals",
    "memory_operations",
    "session_summaries",
    "conversation_summaries",
    "memory_write_events",
    "memory_retrieval_logs",
];

/// Validate the requested character selection against both databases.
///
/// A merge names a character inside the backup and a character that already
/// exists locally; an ignore names a character that must not be restored at all.
/// Silently accepting an unknown id would route the rows nowhere (or drop data
/// the user never selected), so every entry is checked before the first live
/// mutation.
async fn validate_character_selection(
    connection: &mut SqliteConnection,
    import_has_characters: bool,
    merges: &[CharacterMerge],
    ignored: &[String],
) -> Result<(), KokoroError> {
    if merges.is_empty() && ignored.is_empty() {
        return Ok(());
    }
    if !import_has_characters {
        return Err(KokoroError::Validation(
            "backup has no characters table, so nothing can be merged or ignored".to_string(),
        ));
    }

    let mut seen = HashSet::new();
    for merge in merges {
        let imported_id = merge.imported_id.trim();
        let target_id = merge.target_id.trim();
        if imported_id.is_empty() || target_id.is_empty() {
            return Err(KokoroError::Validation(
                "character merge ids cannot be empty".to_string(),
            ));
        }
        if !seen.insert(imported_id.to_string()) {
            return Err(KokoroError::Validation(format!(
                "character '{imported_id}' is mapped more than once"
            )));
        }
        if imported_id == target_id {
            return Err(KokoroError::Validation(format!(
                "character '{imported_id}' cannot be merged into itself"
            )));
        }

        let imported_exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM import_db.characters WHERE id = ?)")
                .bind(imported_id)
                .fetch_one(&mut *connection)
                .await?;
        if imported_exists == 0 {
            return Err(KokoroError::Validation(format!(
                "backup does not contain character '{imported_id}'"
            )));
        }

        let target_exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM characters WHERE id = ?)")
                .bind(target_id)
                .fetch_one(&mut *connection)
                .await?;
        if target_exists == 0 {
            return Err(KokoroError::Validation(format!(
                "local character '{target_id}' does not exist"
            )));
        }
    }

    let mut ignored_seen = HashSet::new();
    for ignored_id in ignored {
        let ignored_id = ignored_id.trim();
        if ignored_id.is_empty() {
            return Err(KokoroError::Validation(
                "ignored character ids cannot be empty".to_string(),
            ));
        }
        if !ignored_seen.insert(ignored_id.to_string()) {
            return Err(KokoroError::Validation(format!(
                "character '{ignored_id}' is ignored more than once"
            )));
        }
        if seen.contains(ignored_id) {
            return Err(KokoroError::Validation(format!(
                "character '{ignored_id}' cannot be merged and ignored at the same time"
            )));
        }

        let imported_exists: i64 =
            sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM import_db.characters WHERE id = ?)")
                .bind(ignored_id)
                .fetch_one(&mut *connection)
                .await?;
        if imported_exists == 0 {
            return Err(KokoroError::Validation(format!(
                "backup does not contain character '{ignored_id}'"
            )));
        }
    }
    Ok(())
}

/// Drop one imported character and everything it owns from the attached backup
/// database, so an ignored character contributes no rows to the restore.
///
/// The backup database is a throwaway copy, so this only affects the import.
async fn remove_imported_character(
    connection: &mut SqliteConnection,
    character_id: &str,
) -> Result<u64, KokoroError> {
    let mut removed = 0_u64;
    // Messages are reached through their conversation, so both tables must exist.
    if import_table_exists(connection, "conversation_messages").await?
        && import_table_exists(connection, "conversations").await?
    {
        removed += sqlx::query(
            "DELETE FROM import_db.conversation_messages WHERE conversation_id IN \
             (SELECT id FROM import_db.conversations WHERE character_id = ?)",
        )
        .bind(character_id)
        .execute(&mut *connection)
        .await
        .map_err(|error| {
            KokoroError::Database(format!(
                "failed to drop ignored conversation messages: {error}"
            ))
        })?
        .rows_affected();
    }
    for table in IMPORT_CHARACTER_SCOPED_TABLES {
        if !import_table_exists(connection, table).await? {
            continue;
        }
        removed += sqlx::query(&format!(
            "DELETE FROM import_db.{table} WHERE character_id = ?"
        ))
        .bind(character_id)
        .execute(&mut *connection)
        .await
        .map_err(|error| {
            KokoroError::Database(format!("failed to drop ignored {table} rows: {error}"))
        })?
        .rows_affected();
    }
    if import_table_exists(connection, "characters").await? {
        removed += sqlx::query("DELETE FROM import_db.characters WHERE id = ?")
            .bind(character_id)
            .execute(&mut *connection)
            .await
            .map_err(|error| {
                KokoroError::Database(format!("failed to drop ignored character row: {error}"))
            })?
            .rows_affected();
    }
    Ok(removed)
}

/// Route every row of the listed imported characters to its local target.
///
/// Runs inside the import transaction before any row is copied, so the merge is
/// what the memory, conversation, and summary inserts see.
async fn apply_import_character_merges(
    connection: &mut SqliteConnection,
    merges: &[CharacterMerge],
) -> Result<Vec<(String, u64)>, KokoroError> {
    let mut remapped = Vec::new();
    for merge in merges {
        let imported_id = merge.imported_id.trim();
        let target_id = merge.target_id.trim();
        for table in IMPORT_CHARACTER_SCOPED_TABLES {
            let exists: Option<String> = sqlx::query_scalar(
                "SELECT name FROM import_db.sqlite_master WHERE type = 'table' AND name = ?",
            )
            .bind(table)
            .fetch_optional(&mut *connection)
            .await?;
            if exists.is_none() {
                continue;
            }

            let result = sqlx::query(&format!(
                "UPDATE import_db.{table} SET character_id = ? WHERE character_id = ?"
            ))
            .bind(target_id)
            .bind(imported_id)
            .execute(&mut *connection)
            .await?;
            if result.rows_affected() > 0 {
                remapped.push((format!("{table}:{imported_id}"), result.rows_affected()));
            }
        }
    }
    Ok(remapped)
}

async fn restore_optional_table(
    connection: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    table: &str,
    columns: &[&str],
    overwrite: bool,
) -> Result<(), KokoroError> {
    let import_exists: Option<String> = sqlx::query_scalar(
        "SELECT name FROM import_db.sqlite_master WHERE type = 'table' AND name = ?",
    )
    .bind(table)
    .fetch_optional(&mut **connection)
    .await?;
    let Some(_) = import_exists else {
        return Ok(());
    };

    let import_columns: HashSet<String> =
        sqlx::query(&format!("PRAGMA import_db.table_info({table})"))
            .fetch_all(&mut **connection)
            .await?
            .into_iter()
            .map(|row| row.get::<String, _>("name"))
            .collect();
    let selected_columns = columns
        .iter()
        .copied()
        .filter(|column| import_columns.contains(*column))
        .collect::<Vec<_>>();
    if selected_columns.is_empty() {
        return Ok(());
    }

    if overwrite {
        sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut **connection)
            .await?;
    }

    let column_list = selected_columns.join(", ");
    let insert = if overwrite {
        "INSERT"
    } else {
        "INSERT OR IGNORE"
    };
    sqlx::query(&format!(
        "{insert} INTO {table} ({column_list}) SELECT {column_list} FROM import_db.{table}"
    ))
    .execute(&mut **connection)
    .await
    .map_err(|error| KokoroError::Database(format!("failed to restore {table}: {error}")))?;
    Ok(())
}

async fn restore_optional_backup_tables(
    connection: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    overwrite: bool,
) -> Result<(), KokoroError> {
    restore_optional_table(
        connection,
        "session_summaries",
        &["id", "character_id", "summary", "created_at"],
        overwrite,
    )
    .await?;
    // `conversation_summaries` is handled by the caller instead: a skip import
    // must only accept summaries for conversations that are new locally, and has
    // to shift their message id range together with the imported messages.
    // emotion_snapshots belongs to the removed emotion subsystem. Keep its
    // migration for old databases, but do not restore dead state from backups.
    restore_optional_table(
        connection,
        "memory_write_events",
        &[
            "id",
            "character_id",
            "source",
            "trigger",
            "extracted_count",
            "stored_count",
            "deduplicated_count",
            "invalidated_count",
            "duration_ms",
            "created_at",
        ],
        overwrite,
    )
    .await?;
    restore_optional_table(
        connection,
        "memory_retrieval_logs",
        &[
            "id",
            "character_id",
            "query",
            "semantic_candidates",
            "bm25_candidates",
            "fused_candidates",
            "injected_count",
            "created_at",
            "overlap_count",
            "semantic_only_count",
            "bm25_only_count",
            "filtered_out_count",
        ],
        overwrite,
    )
    .await?;
    Ok(())
}

/// Copy the dream and audit tables for a full overwrite.
///
/// `memories` is replaced wholesale by an overwrite, so every surviving local row
/// that points at a memory id now points at a *different* memory. Tables holding a
/// memory reference must therefore be cleared even when the backup predates them;
/// otherwise stale dream proposals and audit rows silently re-attach to the
/// imported memories. Tables without a memory reference (`memory_dream_jobs`) are
/// only touched when the backup actually carries them.
async fn restore_memory_aux_tables_overwrite(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    debug_log: &mut Vec<String>,
) -> Result<(), KokoroError> {
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
        .fetch_optional(&mut **transaction)
        .await?;
        if import_has_table.is_none() && !MEMORY_REFERENCING_TABLES.contains(&table) {
            continue;
        }
        let cleared = sqlx::query(&format!("DELETE FROM {table}"))
            .execute(&mut **transaction)
            .await?;
        if import_has_table.is_none() {
            tracing::info!(
                target: "backup",
                "[Backup] Cleared {} stale local {table} row(s): backup has no counterparts",
                cleared.rows_affected()
            );
            debug_log.push(format!(
                "cleared local {table} (absent from backup): {}",
                cleared.rows_affected()
            ));
            continue;
        }
        sqlx::query(&format!(
            "INSERT INTO {table} SELECT * FROM import_db.{table}"
        ))
        .execute(&mut **transaction)
        .await?;
    }
    Ok(())
}

const CONVERSATION_SUMMARY_COLUMNS: &[&str] = &[
    "id",
    "conversation_id",
    "character_id",
    "version",
    "start_message_id",
    "end_message_id",
    "summary",
    "status",
    "failure_count",
    "created_at",
    "updated_at",
];

/// Restore conversation summaries for the active conflict strategy.
///
/// An overwrite replaces every conversation, so the summaries are restored
/// verbatim. A skip import keeps the local conversations, so only summaries that
/// belong to a conversation which is new locally may be copied, and their message
/// id range is shifted by the same offset that was applied to the imported
/// messages so the range keeps pointing at its own conversation.
async fn restore_conversation_summaries(
    connection: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    overwrite: bool,
) -> Result<i64, KokoroError> {
    if overwrite {
        restore_optional_table(
            connection,
            "conversation_summaries",
            CONVERSATION_SUMMARY_COLUMNS,
            true,
        )
        .await?;
        return Ok(0);
    }

    let import_exists: Option<String> = sqlx::query_scalar(
        "SELECT name FROM import_db.sqlite_master \
         WHERE type IN ('table', 'view') AND name = 'conversation_summaries'",
    )
    .fetch_optional(&mut **connection)
    .await?;
    if import_exists.is_none() {
        return Ok(0);
    }

    let result = sqlx::query(
        "INSERT OR IGNORE INTO conversation_summaries \
         (conversation_id, character_id, version, start_message_id, end_message_id, \
          summary, status, failure_count, created_at, updated_at) \
         SELECT summary.conversation_id, summary.character_id, summary.version, \
                summary.start_message_id + (SELECT offset FROM temp.backup_skip_message_offset), \
                summary.end_message_id + (SELECT offset FROM temp.backup_skip_message_offset), \
                summary.summary, summary.status, summary.failure_count, \
                summary.created_at, summary.updated_at \
         FROM import_db.conversation_summaries summary \
         INNER JOIN temp.backup_skip_new_conversations scope \
                 ON scope.id = summary.conversation_id \
         WHERE EXISTS (SELECT 1 FROM conversations local WHERE local.id = summary.conversation_id)",
    )
    .execute(&mut **connection)
    .await
    .map_err(|error| {
        KokoroError::Database(format!("failed to restore conversation summaries: {error}"))
    })?;
    Ok(result.rows_affected() as i64)
}

/// Copy messages that belong to conversations which are new locally.
///
/// Imported message ids come from the source machine and routinely collide with
/// unrelated local messages, so they are moved above every local id instead of
/// being dropped by `INSERT OR IGNORE`. The ordering of the imported rows — and
/// therefore the ordering inside each conversation — is preserved.
async fn insert_skip_conversation_messages(
    connection: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, KokoroError> {
    let import_exists: Option<String> = sqlx::query_scalar(
        "SELECT name FROM import_db.sqlite_master \
         WHERE type IN ('table', 'view') AND name = 'conversation_messages'",
    )
    .fetch_optional(&mut **connection)
    .await?;
    if import_exists.is_none() {
        return Ok(0);
    }
    // An archive entry that is not shaped like the live table (an exotic view, a
    // foreign database) must not abort the whole restore.
    let columns = table_columns(connection, Some("import_db"), "conversation_messages").await?;
    if ![
        "id",
        "conversation_id",
        "role",
        "content",
        "metadata",
        "created_at",
    ]
    .iter()
    .all(|column| columns.contains(*column))
    {
        return Ok(0);
    }

    let result = sqlx::query(
        "INSERT INTO conversation_messages \
         (id, conversation_id, role, content, metadata, created_at) \
         SELECT message.id + (SELECT offset FROM temp.backup_skip_message_offset), \
                message.conversation_id, message.role, message.content, \
                message.metadata, message.created_at \
         FROM import_db.conversation_messages message \
         INNER JOIN temp.backup_skip_new_conversations scope \
                 ON scope.id = message.conversation_id \
         WHERE EXISTS (SELECT 1 FROM conversations local WHERE local.id = message.conversation_id) \
         ORDER BY message.id",
    )
    .execute(&mut **connection)
    .await
    .map_err(|error| {
        KokoroError::Database(format!("failed to restore conversation messages: {error}"))
    })?;
    Ok(result.rows_affected() as i64)
}

/// Outcome of resolving skip-import memory id conflicts.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct SkipMemoryConflicts {
    /// Imported memories whose id is already used locally by a *different* memory.
    pub conflicting_ids: i64,
    /// Imported relation rows dropped because they would point at a local memory.
    pub dropped_relations: i64,
}

async fn table_columns(
    connection: &mut SqliteConnection,
    schema: Option<&str>,
    table: &str,
) -> Result<HashSet<String>, KokoroError> {
    let pragma = match schema {
        Some(schema) => format!("PRAGMA {schema}.table_info({table})"),
        None => format!("PRAGMA table_info({table})"),
    };
    let rows = sqlx::query(&pragma).fetch_all(&mut *connection).await?;
    Ok(rows
        .into_iter()
        .map(|row| row.get::<String, _>("name"))
        .collect())
}

async fn import_table_exists(
    connection: &mut SqliteConnection,
    table: &str,
) -> Result<bool, KokoroError> {
    let found: Option<String> = sqlx::query_scalar(
        "SELECT name FROM import_db.sqlite_master WHERE type IN ('table', 'view') AND name = ?",
    )
    .bind(table)
    .fetch_optional(&mut *connection)
    .await?;
    Ok(found.is_some())
}

/// Skip imports keep local rows with the same integer primary key, so an imported
/// row that references such a key must never be copied: it would silently attach
/// to a *different* local memory once the ids are reused.
///
/// A shared id is only a conflict when the local and the imported row really are
/// different memories (different content or owner). Re-importing an unchanged
/// backup therefore keeps working, while genuinely stale relations are dropped by
/// the caller instead of failing the whole restore. When identity cannot be
/// proven — an import or live schema without a `content` column — every shared id
/// is treated as a conflict.
pub(crate) async fn resolve_skip_memory_conflicts(
    connection: &mut SqliteConnection,
) -> Result<SkipMemoryConflicts, KokoroError> {
    sqlx::query("CREATE TEMP TABLE IF NOT EXISTS backup_skip_memory_ids (id INTEGER PRIMARY KEY)")
        .execute(&mut *connection)
        .await?;
    sqlx::query("DELETE FROM temp.backup_skip_memory_ids")
        .execute(&mut *connection)
        .await?;

    let live_columns = table_columns(connection, None, "memories").await?;
    let import_columns = table_columns(connection, Some("import_db"), "memories").await?;
    let mut identity = Vec::new();
    if live_columns.contains("content") && import_columns.contains("content") {
        identity.push("local.content IS NOT imported.content");
    }
    if live_columns.contains("character_id") && import_columns.contains("character_id") {
        identity.push("local.character_id IS NOT imported.character_id");
    }
    // Without comparable identity columns any shared id may belong to another
    // memory, so fall back to treating the whole overlap as conflicting.
    let identity_predicate = if identity.is_empty() {
        "1".to_string()
    } else {
        identity.join(" OR ")
    };

    sqlx::query(&format!(
        "INSERT INTO temp.backup_skip_memory_ids (id) \
         SELECT imported.id FROM import_db.memories imported \
         INNER JOIN memories local ON local.id = imported.id \
         WHERE {identity_predicate}"
    ))
    .execute(&mut *connection)
    .await?;

    let conflicting_ids: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM temp.backup_skip_memory_ids")
            .fetch_one(&mut *connection)
            .await?;

    // Count only for reporting: the skip inserts below exclude these rows.
    let mut dropped_relations = 0_i64;
    for (table, predicate) in [
        (
            "memory_candidates",
            "applied_memory_id IS NOT NULL AND applied_memory_id IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
        (
            "memory_evidence",
            "memory_id IS NOT NULL AND memory_id IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
        (
            "memory_operations",
            "memory_id IS NOT NULL AND memory_id IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
        (
            "memory_dream_proposals",
            "target_memory_id IS NOT NULL AND target_memory_id IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
    ] {
        if !import_table_exists(connection, table).await? {
            continue;
        }
        let count: i64 =
            sqlx::query_scalar(&format!("SELECT COUNT(*) FROM import_db.{table} WHERE {predicate}"))
                .fetch_one(&mut *connection)
                .await?;
        dropped_relations += count;
    }
    if import_table_exists(connection, "memories").await? {
        let linked: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM import_db.memories memory \
             WHERE EXISTS (\
                 SELECT 1 FROM json_each(\
                     CASE WHEN json_valid(memory.supersedes) THEN memory.supersedes ELSE '[]' END\
                 ) link WHERE CAST(link.value AS INTEGER) IN (SELECT id FROM temp.backup_skip_memory_ids)\
             )",
        )
        .fetch_one(&mut *connection)
        .await
        .unwrap_or(0);
        dropped_relations += linked;
    }

    Ok(SkipMemoryConflicts {
        conflicting_ids,
        dropped_relations,
    })
}

/// Skip imports keep local conversations with the same text primary key, so
/// imported messages and summaries must never be attached to them.
///
/// Instead of rejecting the whole restore, this records the imported
/// conversations that are genuinely new locally (only those are copied) and the
/// id offset that moves imported message ids above every local id, so a new
/// conversation keeps its full history instead of colliding with local message
/// primary keys.
pub(crate) async fn prepare_skip_conversation_scope(
    connection: &mut SqliteConnection,
) -> Result<i64, KokoroError> {
    sqlx::query(
        "CREATE TEMP TABLE IF NOT EXISTS backup_skip_new_conversations (id TEXT PRIMARY KEY)",
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query("DELETE FROM temp.backup_skip_new_conversations")
        .execute(&mut *connection)
        .await?;
    if import_table_exists(connection, "conversations").await? {
        sqlx::query(
            "INSERT OR IGNORE INTO temp.backup_skip_new_conversations (id) \
             SELECT imported.id FROM import_db.conversations imported \
             WHERE NOT EXISTS (SELECT 1 FROM conversations local WHERE local.id = imported.id)",
        )
        .execute(&mut *connection)
        .await?;
    }

    sqlx::query(
        "CREATE TEMP TABLE IF NOT EXISTS backup_skip_message_offset (offset INTEGER NOT NULL)",
    )
    .execute(&mut *connection)
    .await?;
    sqlx::query("DELETE FROM temp.backup_skip_message_offset")
        .execute(&mut *connection)
        .await?;
    sqlx::query(
        "INSERT INTO temp.backup_skip_message_offset (offset) \
         SELECT COALESCE(MAX(id), 0) FROM conversation_messages",
    )
    .execute(&mut *connection)
    .await?;

    let new_conversations: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM temp.backup_skip_new_conversations")
            .fetch_one(&mut *connection)
            .await?;
    Ok(new_conversations)
}

/// Insert the dream and audit relations for a skip import.
///
/// Every relation that names a conflicting memory id is excluded: those ids now
/// belong to the local row that skip mode preserved, so copying the row would
/// attach imported data to a memory it never described. Everything else is
/// imported normally (`INSERT OR IGNORE` still drops rows whose own id is taken).
async fn insert_skip_memory_relations(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<(), KokoroError> {
    for (table, safety_filter) in [
        (
            "memory_candidates",
            "applied_memory_id IS NULL OR applied_memory_id NOT IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
        (
            "memory_evidence",
            "memory_id IS NULL OR memory_id NOT IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
        ("memory_dream_jobs", "1"),
        (
            "memory_dream_proposals",
            "((target_memory_id IS NULL OR target_memory_id NOT IN (SELECT id FROM temp.backup_skip_memory_ids)) \
              AND NOT EXISTS (SELECT 1 FROM json_each(\
                  CASE WHEN json_valid(source_memory_ids) THEN source_memory_ids ELSE '[]' END) source \
                  WHERE CAST(source.value AS INTEGER) IN (SELECT id FROM temp.backup_skip_memory_ids)))",
        ),
        (
            "memory_operations",
            "memory_id IS NULL OR memory_id NOT IN (SELECT id FROM temp.backup_skip_memory_ids)",
        ),
    ] {
        if !import_table_exists(transaction, table).await? {
            continue;
        }
        sqlx::query(&format!(
            "INSERT OR IGNORE INTO {table} SELECT * FROM import_db.{table} WHERE {safety_filter}"
        ))
        .execute(&mut **transaction)
        .await
        .map_err(|error| {
            KokoroError::Database(format!("failed to restore {table} relations: {error}"))
        })?;
    }
    Ok(())
}

/// Skip-mode memory insert.
///
/// `INSERT OR IGNORE` keeps the local row for every reused id, and a `supersedes`
/// link that names a conflicting id is cleared: that id belongs to a local memory
/// the imported row never replaced.
const MEMORY_INSERT_SKIP_SQL: &str = "INSERT OR IGNORE INTO memories \
     (id, content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from, \
      memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
      source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at) \
     SELECT id, content, embedding, created_at, updated_at, importance, character_id, tier, consolidated_from, \
            memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, evidence_count, \
            source_kind, source_refs, \
            CASE \
                WHEN supersedes IS NULL THEN NULL \
                WHEN EXISTS (SELECT 1 FROM json_each(\
                         CASE WHEN json_valid(supersedes) THEN supersedes ELSE '[]' END) link \
                     WHERE CAST(link.value AS INTEGER) IN (SELECT id FROM temp.backup_skip_memory_ids)) \
                    THEN '[]' \
                ELSE supersedes \
            END, \
            canonical_hash, last_dreamed_at FROM import_db.memories";

/// Delete memories that no local character owns, together with the rows that
/// reference them.
///
/// A memory is only reachable through its owner: the memory panel lists them per
/// character and retrieval filters by `character_id`. Rows left without an owner
/// — an imported pre-character-era backup, a character id the local database
/// never had — are therefore unreachable garbage, and are removed instead of
/// being reported to the user.
async fn purge_orphaned_memories(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> Result<i64, KokoroError> {
    const ORPHANED_MEMORY: &str =
        "NOT EXISTS (SELECT 1 FROM characters ch WHERE ch.id = memories.character_id)";

    for (table, column) in [
        ("memory_candidates", "applied_memory_id"),
        ("memory_evidence", "memory_id"),
        ("memory_operations", "memory_id"),
    ] {
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE {column} IN (SELECT id FROM memories WHERE {ORPHANED_MEMORY})"
        ))
        .execute(&mut **transaction)
        .await
        .map_err(|error| {
            KokoroError::Database(format!("failed to purge orphaned {table} rows: {error}"))
        })?;
    }
    sqlx::query(
        "DELETE FROM memory_dream_proposals \
         WHERE target_memory_id IN (SELECT id FROM memories WHERE NOT EXISTS \
                   (SELECT 1 FROM characters ch WHERE ch.id = memories.character_id)) \
            OR EXISTS (SELECT 1 FROM json_each(\
                   CASE WHEN json_valid(source_memory_ids) THEN source_memory_ids ELSE '[]' END) source \
               WHERE CAST(source.value AS INTEGER) IN (SELECT id FROM memories WHERE NOT EXISTS \
                   (SELECT 1 FROM characters ch WHERE ch.id = memories.character_id)))",
    )
    .execute(&mut **transaction)
    .await
    .map_err(|error| {
        KokoroError::Database(format!("failed to purge orphaned dream proposals: {error}"))
    })?;

    for table in crate::commands::characters::CHARACTER_OWNED_TABLES {
        if *table == "memories" {
            continue;
        }
        sqlx::query(&format!(
            "DELETE FROM {table} WHERE character_id NOT IN (SELECT id FROM characters)"
        ))
        .execute(&mut **transaction)
        .await
        .map_err(|error| {
            KokoroError::Database(format!("failed to purge orphaned {table} rows: {error}"))
        })?;
    }

    let purged = sqlx::query(
        "DELETE FROM memories WHERE NOT EXISTS \
                              (SELECT 1 FROM characters ch WHERE ch.id = memories.character_id)",
    )
    .execute(&mut **transaction)
    .await
    .map_err(|error| {
        KokoroError::Database(format!("failed to purge orphaned memories: {error}"))
    })?;
    Ok(purged.rows_affected() as i64)
}

/// Replace derived character activation state only for a full overwrite. The
/// state is optional: malformed or stale runtime JSON is discarded so it cannot
/// prevent the authoritative characters, conversations, and memories from
/// being restored.
async fn restore_committed_runtime(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    overwrite: bool,
) -> Result<(), KokoroError> {
    if !overwrite {
        return Ok(());
    }

    sqlx::query(
        "CREATE TABLE IF NOT EXISTS character_activation_runtime (\
            singleton INTEGER PRIMARY KEY CHECK(singleton = 1),\
            revision INTEGER NOT NULL,\
            runtime_json TEXT NOT NULL\
         )",
    )
    .execute(&mut **transaction)
    .await?;
    sqlx::query("DELETE FROM character_activation_runtime WHERE singleton = 1")
        .execute(&mut **transaction)
        .await?;

    let import_exists: Option<String> = sqlx::query_scalar(
        "SELECT name FROM import_db.sqlite_master WHERE type = 'table' AND name = 'character_activation_runtime'",
    )
    .fetch_optional(&mut **transaction)
    .await?;
    if import_exists.is_none() {
        return Ok(());
    }

    let Some((revision, runtime_json)) = sqlx::query_as::<_, (i64, String)>(
        "SELECT revision, runtime_json FROM import_db.character_activation_runtime WHERE singleton = 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    else {
        return Ok(());
    };
    let parsed: CommittedCharacterRuntime = match serde_json::from_str(&runtime_json) {
        Ok(value) => value,
        Err(error) => {
            tracing::warn!(target: "backup", "discarding invalid imported character runtime: {error}");
            return Ok(());
        }
    };
    let parsed_revision = match i64::try_from(parsed.revision) {
        Ok(value) => value,
        Err(_) => {
            tracing::warn!(target: "backup", "discarding imported character runtime with out-of-range revision");
            return Ok(());
        }
    };
    if revision < 0 || revision != parsed_revision || parsed.runtime.character_id.trim().is_empty()
    {
        tracing::warn!(target: "backup", "discarding inconsistent imported character runtime");
        return Ok(());
    }

    let character_exists: i64 =
        sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM characters WHERE id = ?)")
            .bind(&parsed.runtime.character_id)
            .fetch_one(&mut **transaction)
            .await?;
    if character_exists == 0 {
        tracing::warn!(
            target: "backup",
            "discarding imported character runtime for missing character {}",
            parsed.runtime.character_id
        );
        return Ok(());
    }

    if let Some(conversation_id) = parsed.runtime.current_conversation_id.as_deref() {
        let owner: Option<String> =
            sqlx::query_scalar("SELECT character_id FROM conversations WHERE id = ?")
                .bind(conversation_id)
                .fetch_optional(&mut **transaction)
                .await?;
        if owner.as_deref() != Some(parsed.runtime.character_id.as_str()) {
            tracing::warn!(
                target: "backup",
                "discarding imported character runtime for unavailable conversation {}",
                conversation_id
            );
            return Ok(());
        }
    }

    sqlx::query(
        "INSERT INTO character_activation_runtime (singleton, revision, runtime_json) VALUES (1, ?, ?)",
    )
    .bind(revision)
    .bind(runtime_json)
    .execute(&mut **transaction)
    .await?;
    Ok(())
}

/// Open a read-only sqlx pool to a given DB file.
async fn open_readonly_pool(path: &Path) -> Result<SqlitePool, KokoroError> {
    if open_regular_non_redirected_file(path, "database")?.is_none() {
        return Err(KokoroError::NotFound(format!(
            "database does not exist: {}",
            path.display()
        )));
    }
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
    if !matches!(
        open_regular_non_redirected_file(path, "database"),
        Ok(Some(_))
    ) {
        return BackupStats {
            memories: 0,
            conversations: 0,
            messages: 0,
            configs: 0,
        };
    }
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
    if open_regular_non_redirected_file(database, "database")?.is_none() {
        return Ok(());
    }
    let pool = open_readonly_pool(database).await?;
    let references = load_template_references(&pool).await?;
    let instance_avatars = sqlx::query(
        "SELECT id, avatar_path FROM characters \
         WHERE avatar_path LIKE 'character-instance-resource://%/avatar.png'",
    )
    .fetch_all(&pool)
    .await?;
    pool.close().await;
    let resolver = LocalCatalogPackageResolver::new(app_data.join("characters"));
    for (template_id, template_version) in references {
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
            ensure_non_redirected_parent_chain(
                &package.package_dir,
                source.parent().unwrap_or(&package.package_dir),
            )?;
            let mut input = open_regular_non_redirected_file(&source, "character resource")?
                .ok_or_else(|| {
                    KokoroError::Validation(format!(
                        "character resource disappeared during export: {}",
                        source.display()
                    ))
                })?;
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
        ensure_non_redirected_parent_chain(
            app_data,
            source.parent().ok_or_else(|| {
                KokoroError::Validation("managed avatar source has no parent".to_string())
            })?,
        )?;
        let mut input = open_regular_non_redirected_file(&source, "managed avatar resource")?
            .ok_or_else(|| {
                KokoroError::Validation(format!(
                    "managed avatar resource disappeared during export: {}",
                    source.display()
                ))
            })?;
        std::io::copy(&mut input, zip).map_err(KokoroError::from)?;
    }
    Ok(())
}

fn is_backup_secret_key(key: &str) -> bool {
    let normalized = key.trim().to_ascii_lowercase().replace('-', "_");
    if normalized.ends_with("_env") {
        return false;
    }
    let compact = normalized.replace('_', "");
    normalized == "token"
        || normalized.ends_with("_token")
        || normalized == "secret"
        || normalized.ends_with("_secret")
        || normalized.contains("api_key")
        || compact.contains("apikey")
        || compact.ends_with("token")
        || compact.ends_with("secret")
        || normalized.contains("access_token")
        || compact.contains("accesstoken")
        || normalized.contains("authorization")
        || normalized.contains("password")
        || normalized.contains("credential")
}

/// Remove secret values from arbitrary provider configuration JSON while
/// preserving the surrounding shape so data-only backups remain importable.
fn sanitize_backup_json(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            let keys = object.keys().cloned().collect::<Vec<_>>();
            for key in keys {
                if key.eq_ignore_ascii_case("env") {
                    if let Some(child) = object.get_mut(&key) {
                        sanitize_backup_json(child);
                    }
                    continue;
                }
                if is_backup_secret_key(&key) {
                    object.remove(&key);
                    continue;
                }
                if let Some(child) = object.get_mut(&key) {
                    sanitize_backup_json(child);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                sanitize_backup_json(item);
            }
        }
        serde_json::Value::Null
        | serde_json::Value::Bool(_)
        | serde_json::Value::Number(_)
        | serde_json::Value::String(_) => {}
    }
}

fn sanitize_backup_config(content: &str, filename: &str) -> Result<String, KokoroError> {
    let mut value: serde_json::Value = serde_json::from_str(content).map_err(|error| {
        KokoroError::Validation(format!("invalid JSON config {filename}: {error}"))
    })?;
    sanitize_backup_json(&mut value);
    serde_json::to_string_pretty(&value).map_err(|error| {
        KokoroError::Internal(format!(
            "failed to serialize sanitized config {filename}: {error}"
        ))
    })
}

fn merge_local_backup_secrets(imported: &mut serde_json::Value, local: &serde_json::Value) {
    match (imported, local) {
        (serde_json::Value::Object(imported_object), serde_json::Value::Object(local_object)) => {
            for (key, local_value) in local_object {
                if key.eq_ignore_ascii_case("env") {
                    if let Some(imported_value) = imported_object.get_mut(key) {
                        merge_local_backup_secrets(imported_value, local_value);
                    } else {
                        imported_object.insert(key.clone(), local_value.clone());
                    }
                    continue;
                }
                if is_backup_secret_key(key) {
                    if !imported_object.contains_key(key) {
                        imported_object.insert(key.clone(), local_value.clone());
                    }
                    continue;
                }
                if let Some(imported_value) = imported_object.get_mut(key) {
                    merge_local_backup_secrets(imported_value, local_value);
                }
            }
        }
        (serde_json::Value::Array(imported_items), serde_json::Value::Array(local_items)) => {
            for imported_item in imported_items {
                let local_item = local_items.iter().find(|local_item| {
                    ["id", "provider_id", "name"].iter().any(|key| {
                        imported_item.get(*key).and_then(serde_json::Value::as_str)
                            == local_item.get(*key).and_then(serde_json::Value::as_str)
                            && imported_item.get(*key).is_some()
                    })
                });
                if let Some(local_item) = local_item {
                    merge_local_backup_secrets(imported_item, local_item);
                }
            }
        }
        _ => {}
    }
}

fn prepare_backup_config_for_install(
    app_data: &Path,
    filename: &str,
    content: &str,
) -> Result<String, KokoroError> {
    let mut imported: serde_json::Value = serde_json::from_str(content).map_err(|error| {
        KokoroError::Validation(format!("invalid JSON config {filename}: {error}"))
    })?;
    let original = imported.clone();
    sanitize_backup_json(&mut imported);

    let local_path = app_data.join(filename);
    if let Ok(local_content) = fs::read_to_string(&local_path) {
        if let Ok(local) = serde_json::from_str::<serde_json::Value>(&local_content) {
            merge_local_backup_secrets(&mut imported, &local);
        }
    }

    if imported == original {
        return Ok(content.to_string());
    }

    serde_json::to_string_pretty(&imported).map_err(|error| {
        KokoroError::Internal(format!(
            "failed to serialize imported config {filename}: {error}"
        ))
    })
}

// ── Commands ─────────────────────────────────────────

#[tauri::command]
pub async fn export_data(
    app: AppHandle,
    export_path: String,
    _characters_json: Option<String>,
    options: Option<ExportOptions>,
) -> Result<ExportResult, KokoroError> {
    let _database_operation_guard = acquire_database_operation_read_guard().await;
    let app_data = app_data_dir(&app)?;
    ensure_non_redirected_parent_chain(&app_data, &app_data)?;
    let db = db_path(&app_data);

    let out_path = PathBuf::from(&export_path);
    validate_export_target(&app_data, &out_path)?;
    let (mut export_guard, file) = create_atomic_export_file(&out_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let zip_options =
        SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    // 1. Snapshot the database first: the archived bytes are the source of truth
    // for both the archive and the counts reported back to the user.
    let snapshot = create_consistent_database_snapshot(&db).await?;
    let mut stats = snapshot
        .as_ref()
        .map(|snapshot| snapshot.stats.clone())
        .unwrap_or_default();
    let mut config_count: usize = 0;

    // 2. manifest.json
    let options_value = options.unwrap_or_default();
    let manifest = BackupManifest {
        version: BACKUP_FORMAT_VERSION.to_string(),
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

    // 3. kokoro.db — the consistent SQLite snapshot created above.
    if let Some(snapshot) = snapshot {
        zip.start_file("kokoro.db", zip_options)
            .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
        zip.write_all(&snapshot.bytes).map_err(KokoroError::from)?;
    }

    // 4. Optional character packages. Character rows themselves are already in SQLite.
    if options_value.include_character_resources {
        write_character_resources(&mut zip, zip_options, &app_data, &db).await?;
    }

    // 5. configs/
    for name in CONFIG_FILES {
        let cfg_path = app_data.join(name);
        if let Some(mut input) = open_regular_non_redirected_file(&cfg_path, "config")? {
            let content = String::from_utf8(read_limited_bytes(
                &mut input,
                MAX_BACKUP_CONFIG_BYTES,
                "config",
            )?)
            .map_err(|error| KokoroError::Validation(format!("config is not UTF-8: {error}")))?;
            let content = sanitize_backup_config(&content, name)?;
            let entry = format!("configs/{}", name);
            zip.start_file(&entry, zip_options)
                .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
            zip.write_all(content.as_bytes())
                .map_err(KokoroError::from)?;
            config_count += 1;
        }
    }

    let file = zip
        .finish()
        .map_err(|e| KokoroError::Internal(format!("ZIP finish error: {}", e)))?;
    commit_atomic_export(&mut export_guard, file, &out_path)?;

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
    let _database_operation_guard = acquire_database_operation_read_guard().await;
    ensure_non_redirected_parent_chain(app_data, app_data)?;
    let db = db_path(app_data);

    validate_export_target(app_data, out_path)?;
    let (mut export_guard, file) = create_atomic_export_file(out_path)?;
    let mut zip = zip::ZipWriter::new(file);
    let options = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let snapshot = create_consistent_database_snapshot(&db).await?;
    let mut stats = snapshot
        .as_ref()
        .map(|snapshot| snapshot.stats.clone())
        .unwrap_or_default();
    let mut config_count: usize = 0;

    let manifest = BackupManifest {
        version: BACKUP_FORMAT_VERSION.to_string(),
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

    if let Some(snapshot) = snapshot {
        zip.start_file("kokoro.db", options)
            .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
        zip.write_all(&snapshot.bytes).map_err(KokoroError::from)?;
    }

    for name in CONFIG_FILES {
        let cfg_path = app_data.join(name);
        if let Some(mut input) = open_regular_non_redirected_file(&cfg_path, "config")? {
            let content = String::from_utf8(read_limited_bytes(
                &mut input,
                MAX_BACKUP_CONFIG_BYTES,
                "config",
            )?)
            .map_err(|error| KokoroError::Validation(format!("config is not UTF-8: {error}")))?;
            let content = sanitize_backup_config(&content, name)?;
            let entry = format!("configs/{}", name);
            zip.start_file(&entry, options)
                .map_err(|e| KokoroError::Internal(format!("ZIP error: {}", e)))?;
            zip.write_all(content.as_bytes())
                .map_err(KokoroError::from)?;
            config_count += 1;
        }
    }

    let file = zip
        .finish()
        .map_err(|e| KokoroError::Internal(format!("ZIP finish error: {}", e)))?;
    commit_atomic_export(&mut export_guard, file, out_path)?;

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
    validate_archive_path_uniqueness(&mut archive)?;

    // Read manifest
    let manifest: BackupManifest = {
        let mut entry = archive.by_name("manifest.json").map_err(|_| {
            KokoroError::Validation("Missing manifest.json in backup file".to_string())
        })?;
        let buf = String::from_utf8(read_limited_bytes(
            &mut entry,
            MAX_BACKUP_MANIFEST_BYTES,
            "backup manifest",
        )?)
        .map_err(|error| KokoroError::Validation(format!("invalid UTF-8 manifest: {error}")))?;
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
                let filename = name.trim_start_matches("configs/");
                if !LEGACY_IGNORED_CONFIG_FILES.contains(&filename) {
                    config_files.push(filename.to_string());
                }
            }
        }
    }
    let has_configs = !config_files.is_empty();

    // If DB present, extract to temp and count rows
    let (stats, characters) = if has_database {
        let tmp_guard = create_scoped_temp_dir("kokoro_import_preview")?;
        let tmp_dir_path = tmp_guard.path();
        let tmp_db = tmp_dir_path.join("preview.db");

        {
            let mut entry = archive
                .by_name("kokoro.db")
                .map_err(|e| KokoroError::Internal(format!("Failed to read DB from ZIP: {}", e)))?;
            let bytes = read_limited_bytes(&mut entry, MAX_BACKUP_DATABASE_BYTES, "database")?;
            let mut out = fs::File::create(&tmp_db).map_err(KokoroError::from)?;
            out.write_all(&bytes).map_err(KokoroError::from)?;
        }

        let stats = gather_stats(&tmp_db).await;
        let characters = read_backup_characters(&tmp_db).await;
        (stats, characters)
    } else {
        (
            BackupStats {
                memories: 0,
                conversations: 0,
                messages: 0,
                configs: 0,
            },
            Vec::new(),
        )
    };

    Ok(ImportPreview {
        manifest,
        has_database,
        has_configs,
        config_files,
        stats,
        characters,
    })
}

/// List the character instances stored in a backup database, with the amount of
/// data each of them owns. Backups written before characters moved into SQLite
/// have no `characters` table and yield an empty list, which the import treats as
/// "nothing to map".
async fn read_backup_characters(path: &Path) -> Vec<BackupCharacterSummary> {
    let Ok(pool) = open_readonly_pool(path).await else {
        return Vec::new();
    };
    let characters = async {
        if !table_exists(&pool, "characters").await? {
            return Ok(Vec::new());
        }
        let rows = sqlx::query(
            "SELECT character.id AS id, character.name AS name, \
                    (SELECT COUNT(*) FROM memories memory WHERE memory.character_id = character.id) AS memory_count, \
                    (SELECT COUNT(*) FROM conversations conversation WHERE conversation.character_id = character.id) AS conversation_count \
             FROM characters character ORDER BY character.name COLLATE NOCASE ASC, character.id ASC",
        )
        .fetch_all(&pool)
        .await?;
        let mut summaries = Vec::with_capacity(rows.len());
        for row in rows {
            summaries.push(BackupCharacterSummary {
                id: row.get::<String, _>("id"),
                name: row.get::<String, _>("name"),
                memory_count: row.get::<i64, _>("memory_count"),
                conversation_count: row.get::<i64, _>("conversation_count"),
            });
        }
        Ok::<_, sqlx::Error>(summaries)
    }
    .await;
    pool.close().await;
    match characters {
        Ok(summaries) => summaries,
        Err(error) => {
            tracing::warn!(target: "backup", "[Backup] Failed to read backup characters: {error}");
            Vec::new()
        }
    }
}

async fn table_exists(pool: &SqlitePool, table: &str) -> Result<bool, sqlx::Error> {
    let found: Option<String> =
        sqlx::query_scalar("SELECT name FROM sqlite_master WHERE type='table' AND name = ?")
            .bind(table)
            .fetch_optional(pool)
            .await?;
    Ok(found.is_some())
}

#[tauri::command]
pub async fn import_data(
    app: AppHandle,
    file_path: String,
    options: ImportOptions,
) -> Result<ImportResult, KokoroError> {
    let _database_operation_guard = acquire_database_operation_write_guard().await;
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
            let bytes = read_limited_bytes(&mut entry, MAX_BACKUP_DATABASE_BYTES, "database")?;
            let mut out = fs::File::create(tmp_dir.join("import.db")).map_err(KokoroError::from)?;
            out.write_all(&bytes).map_err(KokoroError::from)?;
        }
    }
    // archive is dropped here — safe to .await below
    let extracted_configs: Vec<(String, String)> = staged_configs
        .into_iter()
        .filter(|(filename, _)| should_import_config(filename, options.import_database, has_db))
        .filter(|(filename, _)| {
            options.conflict_strategy != ConflictStrategy::Skip || !app_data.join(filename).exists()
        })
        .collect();

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
    let (mut prepared_characters, import_has_characters) = if has_db {
        let source_pool = open_readonly_pool(&tmp_dir.join("import.db")).await?;
        let catalog_root = app_data.join("characters");
        let local_resolver = LocalCatalogPackageResolver::new(catalog_root.clone());
        let official_resolver = OfficialRegistryPackageResolver::new(catalog_root);
        let references = load_template_references(&source_pool).await?;
        for (template_id, template_version) in references {
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
        let import_has_characters = table_exists(&source_pool, "characters")
            .await
            .unwrap_or(false);
        source_pool.close().await;
        (rows, import_has_characters)
    } else {
        (Vec::new(), false)
    };

    // Phase 2: Async DB operations
    let mut result = ImportResult {
        imported_memories: 0,
        imported_conversations: 0,
        imported_configs: 0,
        imported_characters: 0,
        merged_characters: 0,
        ignored_characters: 0,
        // Kept only for wire compatibility with old frontends; SQLite is authoritative.
        characters_json: None,
        skipped_memories: 0,
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

        // The archive must actually contain a Kokoro database before any live row
        // is touched; an unrelated SQLite file would otherwise fail deep inside
        // the column normalization below.
        if !import_table_exists(&mut conn, "memories").await? {
            return Err(KokoroError::Validation(
                "backup database does not contain a memories table".to_string(),
            ));
        }

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

        // Normalize the additive proposal revision column before copying any
        // dream tables. Older backups do not have it; such proposals retain
        // the empty default and are rejected safely by the approval path.
        let import_proposal_exists: Option<String> = sqlx::query_scalar(
            "SELECT name FROM import_db.sqlite_master WHERE type = 'table' AND name = 'memory_dream_proposals'",
        )
        .fetch_optional(&mut *conn)
        .await?;
        if import_proposal_exists.is_some() {
            let proposal_columns: Vec<String> =
                sqlx::query("PRAGMA import_db.table_info(memory_dream_proposals)")
                    .fetch_all(&mut *conn)
                    .await?
                    .into_iter()
                    .map(|row| row.get::<String, _>("name"))
                    .collect();
            if !proposal_columns
                .iter()
                .any(|column| column == "source_memory_versions")
            {
                sqlx::query(
                    "ALTER TABLE import_db.memory_dream_proposals \
                     ADD COLUMN source_memory_versions TEXT NOT NULL DEFAULT '[]'",
                )
                .execute(&mut *conn)
                .await
                .map_err(|error| {
                    KokoroError::Database(format!(
                        "Failed to normalize import proposal revision column: {error}"
                    ))
                })?;
            }
        }

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
            "character merges: {:?}",
            options
                .character_merges
                .iter()
                .map(|merge| format!("{}->{}", merge.imported_id, merge.target_id))
                .collect::<Vec<_>>()
        ));

        let import_has_conversations = import_table_exists(&mut conn, "conversations").await?;
        let import_conversation_columns: Vec<String> = if import_has_conversations {
            sqlx::query("PRAGMA import_db.table_info(conversations)")
                .fetch_all(&mut *conn)
                .await
                .unwrap_or_default()
                .into_iter()
                .map(|row| row.get::<String, _>("name"))
                .collect()
        } else {
            Vec::new()
        };
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

        // Validate the character selection before opening the transaction so an
        // invalid choice fails without touching the live database.
        validate_character_selection(
            &mut conn,
            import_has_characters,
            &options.character_merges,
            &options.ignored_characters,
        )
        .await?;

        // Every live-table mutation below shares this connection transaction. DDL for
        // the FTS triggers is transactional in SQLite, so any error restores both data
        // and trigger state before the connection is released.
        let mut transaction = conn.begin().await?;

        // Ignored characters contribute nothing: their rows leave the backup
        // database before any copy statement runs, and their instance is not
        // imported either. Dropping first also keeps the routing step below from
        // ever moving rows out of a character the user chose to leave out.
        let ignored_ids: HashSet<String> = options
            .ignored_characters
            .iter()
            .map(|id| id.trim().to_string())
            .collect();
        for ignored_id in &ignored_ids {
            let dropped = remove_imported_character(&mut transaction, ignored_id).await?;
            tracing::info!(
                target: "backup",
                "[Backup] Ignored character '{ignored_id}' ({dropped} row(s) dropped)"
            );
            result
                .debug_log
                .push(format!("ignored {ignored_id}: {dropped} row(s) dropped"));
        }
        result.ignored_characters = prepared_characters
            .iter()
            .filter(|row| ignored_ids.contains(&row.id))
            .count() as i64;
        prepared_characters.retain(|row| !ignored_ids.contains(&row.id));

        // Merging rewrites the remaining imported rows before they are copied, so
        // memories and conversations land on the local character the user picked.
        for (table, count) in
            apply_import_character_merges(&mut transaction, &options.character_merges).await?
        {
            tracing::info!(target: "backup", "[Backup] Merged {count} row(s) of {table}");
            result
                .debug_log
                .push(format!("merged {table} rows: {count}"));
        }

        // A merged character keeps its local instance row; only the others are
        // inserted as new characters.
        let merged_ids: HashSet<&str> = options
            .character_merges
            .iter()
            .map(|merge| merge.imported_id.trim())
            .collect();
        result.merged_characters = prepared_characters
            .iter()
            .filter(|row| merged_ids.contains(row.id.as_str()))
            .count() as i64;
        prepared_characters.retain(|row| !merged_ids.contains(row.id.as_str()));

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

            restore_memory_aux_tables_overwrite(&mut transaction, &mut result.debug_log).await?;

            if import_has_conversations {
                let r = sqlx::query(conversation_insert_sql)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|e| {
                        KokoroError::Database(format!("INSERT conversations failed: {}", e))
                    })?;
                result.imported_conversations = r.rows_affected() as i64;
            }
            result.debug_log.push(format!(
                "inserted conversations: {}",
                result.imported_conversations
            ));

            let import_has_messages =
                import_table_exists(&mut transaction, "conversation_messages").await?;
            if import_has_messages {
                sqlx::query(
                    "INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at)
                     SELECT id, conversation_id, role, content, metadata, created_at FROM import_db.conversation_messages",
                )
                    .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("INSERT conversation_messages failed: {}", e))
                })?;
            }

            // 重建 FTS 索引并恢复触发器
            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
                .execute(&mut *transaction)
                .await?;
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_ai AFTER INSERT ON memories BEGIN INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END").execute(&mut *transaction).await?;
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_ad AFTER DELETE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content); END").execute(&mut *transaction).await?;
            sqlx::query("CREATE TRIGGER IF NOT EXISTS memories_au AFTER UPDATE ON memories BEGIN INSERT INTO memories_fts(memories_fts, rowid, content) VALUES('delete', old.id, old.content); INSERT INTO memories_fts(rowid, content) VALUES (new.id, new.content); END").execute(&mut *transaction).await?;
        } else {
            // Skip preserves local integer IDs and local conversations. Relations
            // that would follow a skipped ID into a *different* local row are
            // dropped instead of being attached; messages are only copied for
            // conversations that are new locally and are re-numbered above every
            // local message id so they cannot collide either.
            let memory_conflicts = resolve_skip_memory_conflicts(&mut transaction).await?;
            let new_conversations = prepare_skip_conversation_scope(&mut transaction).await?;
            result.debug_log.push(format!(
                "skip scope: {} conflicting memory id(s), {} dropped relation row(s), {} new conversation(s)",
                memory_conflicts.conflicting_ids,
                memory_conflicts.dropped_relations,
                new_conversations
            ));

            // skip 模式：先重建 FTS 以防损坏
            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
                .execute(&mut *transaction)
                .await?;

            let r = sqlx::query(MEMORY_INSERT_SKIP_SQL)
                .execute(&mut *transaction)
                .await
                .map_err(|e| {
                    KokoroError::Database(format!("INSERT OR IGNORE memories failed: {}", e))
                })?;
            result.imported_memories = r.rows_affected() as i64;
            // `INSERT OR IGNORE` also drops rows that violate a local constraint
            // (for example a NULL in a column the live schema requires). Surface
            // that instead of reporting a silent shortfall.
            let importable: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM import_db.memories imported \
                 WHERE NOT EXISTS (SELECT 1 FROM memories local WHERE local.id = imported.id)",
            )
            .fetch_one(&mut *transaction)
            .await
            .unwrap_or(result.imported_memories);
            if importable > result.imported_memories {
                result.skipped_memories = importable - result.imported_memories;
                tracing::warn!(
                    target: "backup",
                    "[Backup] Skip import dropped {} memory row(s) that violate the local schema",
                    result.skipped_memories
                );
                result.debug_log.push(format!(
                    "dropped memories that violate the local schema: {}",
                    result.skipped_memories
                ));
            }
            tracing::info!(
                target: "backup",
                "[Backup] Inserted {} memories (skip mode)",
                result.imported_memories
            );
            result.debug_log.push(format!(
                "inserted memories (skip): {}",
                result.imported_memories
            ));

            insert_skip_memory_relations(&mut transaction).await?;

            if import_has_conversations {
                let r = sqlx::query(conversation_insert_skip_sql)
                    .execute(&mut *transaction)
                    .await
                    .map_err(|e| {
                        KokoroError::Database(format!(
                            "INSERT OR IGNORE conversations failed: {}",
                            e
                        ))
                    })?;
                result.imported_conversations = r.rows_affected() as i64;
            }
            result.debug_log.push(format!(
                "inserted conversations (skip): {}",
                result.imported_conversations
            ));

            let restored_messages = insert_skip_conversation_messages(&mut transaction).await?;
            result.debug_log.push(format!(
                "inserted conversation messages (skip): {restored_messages}"
            ));

            sqlx::query("INSERT INTO memories_fts(memories_fts) VALUES('rebuild')")
                .execute(&mut *transaction)
                .await?;
        }

        restore_optional_backup_tables(
            &mut transaction,
            options.conflict_strategy == ConflictStrategy::Overwrite,
        )
        .await?;
        restore_conversation_summaries(
            &mut transaction,
            options.conflict_strategy == ConflictStrategy::Overwrite,
        )
        .await?;

        result.imported_characters = apply_character_rows(
            &mut transaction,
            prepared_characters,
            options.conflict_strategy.as_str(),
        )
        .await?;
        restore_committed_runtime(
            &mut transaction,
            options.conflict_strategy == ConflictStrategy::Overwrite,
        )
        .await?;
        // Characters are restored above, so this is the first point where an
        // imported memory can be attributed to an owner. Anything still without
        // one is unreachable and is removed quietly.
        let purged_memories = purge_orphaned_memories(&mut transaction).await?;
        if purged_memories > 0 {
            tracing::info!(
                target: "backup",
                "[Backup] Purged {} memory row(s) without a local character owner",
                purged_memories
            );
            result
                .debug_log
                .push(format!("purged orphaned memories: {purged_memories}"));
        }
        transaction.commit().await?;

        // Database rows now reference the imported files. Do not let a later
        // connection-level DETACH error remove those committed resources.
        promoted_resources.disarm();
        config_replacement.disarm();

        detach_import_database_best_effort(&mut conn).await;

        // A merge means the user chose a local character to keep using, so make it
        // the active one once every live table has committed.
        for merge in &options.character_merges {
            let target_id = merge.target_id.trim();
            if !target_id.is_empty() {
                crate::ai::context::AIOrchestrator::persist_active_character_id(target_id);
                result
                    .debug_log
                    .push(format!("persisted active_character_id: {target_id}"));
                break;
            }
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
    async fn character_merge_rewrites_import_rows_without_touching_live_rows() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();

        sqlx::query("CREATE TABLE memories (id INTEGER PRIMARY KEY, character_id TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO memories (id, character_id) VALUES (1, 'live-character')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.memories (id INTEGER PRIMARY KEY, character_id TEXT NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO import_db.memories (id, character_id) VALUES (2, 'import-character'), \
             (3, 'other-import-character')",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        apply_import_character_merges(
            &mut connection,
            &[CharacterMerge {
                imported_id: "import-character".to_string(),
                target_id: "live-character".to_string(),
            }],
        )
        .await
        .unwrap();

        let live_character: String =
            sqlx::query_scalar("SELECT character_id FROM memories WHERE id = 1")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        let merged: String =
            sqlx::query_scalar("SELECT character_id FROM import_db.memories WHERE id = 2")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        let untouched: String =
            sqlx::query_scalar("SELECT character_id FROM import_db.memories WHERE id = 3")
                .fetch_one(&mut *connection)
                .await
                .unwrap();

        assert_eq!(live_character, "live-character");
        assert_eq!(merged, "live-character");
        assert_eq!(
            untouched, "other-import-character",
            "only the listed character is routed"
        );
    }

    #[tokio::test]
    async fn character_merge_validation_rejects_unknown_ids() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("INSERT INTO characters (id, name) VALUES ('local-1', 'Local')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE import_db.characters (id TEXT PRIMARY KEY, name TEXT)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO import_db.characters (id, name) VALUES ('remote-1', 'Remote')")
            .execute(&mut *connection)
            .await
            .unwrap();

        let cases = [
            (
                CharacterMerge {
                    imported_id: "missing".to_string(),
                    target_id: "local-1".to_string(),
                },
                "backup does not contain character",
            ),
            (
                CharacterMerge {
                    imported_id: "remote-1".to_string(),
                    target_id: "missing".to_string(),
                },
                "local character",
            ),
            (
                CharacterMerge {
                    imported_id: "remote-1".to_string(),
                    target_id: "remote-1".to_string(),
                },
                "cannot be merged into itself",
            ),
        ];
        for (merge, expected) in cases {
            let error = validate_character_selection(&mut connection, true, &[merge], &[])
                .await
                .unwrap_err();
            assert!(error.to_string().contains(expected), "{error}");
        }

        validate_character_selection(
            &mut connection,
            true,
            &[CharacterMerge {
                imported_id: "remote-1".to_string(),
                target_id: "local-1".to_string(),
            }],
            &[],
        )
        .await
        .unwrap();

        let duplicate = validate_character_selection(
            &mut connection,
            true,
            &[
                CharacterMerge {
                    imported_id: "remote-1".to_string(),
                    target_id: "local-1".to_string(),
                },
                CharacterMerge {
                    imported_id: "remote-1".to_string(),
                    target_id: "local-1".to_string(),
                },
            ],
            &[],
        )
        .await
        .unwrap_err();
        assert!(duplicate.to_string().contains("mapped more than once"));

        let no_characters = validate_character_selection(
            &mut connection,
            false,
            &[CharacterMerge {
                imported_id: "remote-1".to_string(),
                target_id: "local-1".to_string(),
            }],
            &[],
        )
        .await
        .unwrap_err();
        assert!(no_characters.to_string().contains("no characters table"));

        // Ignoring a character is validated with the same rules.
        validate_character_selection(&mut connection, true, &[], &["remote-1".to_string()])
            .await
            .unwrap();

        let ignored_unknown =
            validate_character_selection(&mut connection, true, &[], &["missing".to_string()])
                .await
                .unwrap_err();
        assert!(ignored_unknown
            .to_string()
            .contains("backup does not contain character"));

        let ignored_twice = validate_character_selection(
            &mut connection,
            true,
            &[],
            &["remote-1".to_string(), "remote-1".to_string()],
        )
        .await
        .unwrap_err();
        assert!(ignored_twice.to_string().contains("ignored more than once"));

        let merged_and_ignored = validate_character_selection(
            &mut connection,
            true,
            &[CharacterMerge {
                imported_id: "remote-1".to_string(),
                target_id: "local-1".to_string(),
            }],
            &["remote-1".to_string()],
        )
        .await
        .unwrap_err();
        assert!(merged_and_ignored
            .to_string()
            .contains("merged and ignored at the same time"));
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
    fn official_registry_restore_selects_the_exact_character_version_only() {
        let index = RegistryIndex {
            schema_version: 1,
            registry_version: 1,
            generated_at: None,
            entries: vec![
                RegistryEntry {
                    content_type: "character".to_string(),
                    id: "kokoro".to_string(),
                    name: "Kokoro".to_string(),
                    version: "1.0.0".to_string(),
                    author: "team".to_string(),
                    description: "old".to_string(),
                    preview: Vec::new(),
                    engine_version: ">=0.1.0".to_string(),
                    download_url: "https://example.test/kokoro-1.0.0.zip".to_string(),
                    archive_size: 1,
                    sha256: "a".repeat(64),
                    trust: "official".to_string(),
                    trust_source: OFFICIAL_REGISTRY_URL.to_string(),
                    registry_identity: None,
                    permissions: Vec::new(),
                    recommendations: crate::registry::manifest::RegistryRecommendations {
                        vision: false,
                        memory: false,
                        mcp_servers: Vec::new(),
                        bot_platforms: Vec::new(),
                    },
                },
                RegistryEntry {
                    content_type: "character".to_string(),
                    id: "kokoro".to_string(),
                    name: "Kokoro".to_string(),
                    version: "2.0.0".to_string(),
                    author: "team".to_string(),
                    description: "new".to_string(),
                    preview: Vec::new(),
                    engine_version: ">=0.1.0".to_string(),
                    download_url: "https://example.test/kokoro-2.0.0.zip".to_string(),
                    archive_size: 1,
                    sha256: "b".repeat(64),
                    trust: "official".to_string(),
                    trust_source: OFFICIAL_REGISTRY_URL.to_string(),
                    registry_identity: None,
                    permissions: Vec::new(),
                    recommendations: crate::registry::manifest::RegistryRecommendations {
                        vision: false,
                        memory: false,
                        mcp_servers: Vec::new(),
                        bot_platforms: Vec::new(),
                    },
                },
            ],
        };

        assert_eq!(
            find_official_character_package(&index, "kokoro", "1.0.0")
                .map(|entry| entry.description.as_str()),
            Some("old")
        );
        assert!(find_official_character_package(&index, "kokoro", "3.0.0").is_none());
        assert!(find_official_character_package(&index, "../kokoro", "1.0.0").is_none());
    }

    #[test]
    fn accepts_legal_uppercase_semver_prerelease_and_build_path_segments() {
        assert!(validate_catalog_segment("1.0.0-ALPHA+Build.7", "template version").is_ok());
        assert!(validate_catalog_segment("../escape", "template version").is_err());
    }

    #[test]
    fn rejects_case_folded_duplicate_character_resource_paths_before_staging() {
        let temp = tempfile::tempdir().unwrap();
        let archive_path = temp.path().join("backup.zip");
        let mut bytes = Cursor::new(Vec::new());
        {
            let mut writer = zip::ZipWriter::new(&mut bytes);
            let options = SimpleFileOptions::default();
            let manifest = serde_json::json!({
                "schema_version": 1,
                "engine_version": ">=0.3.0, <0.4.0",
                "id": "kokoro",
                "version": "1.0.0",
                "name": "Kokoro",
                "description": "A character",
                "author": "Kokoro",
                "license": "MIT",
                "persona": "Be helpful.",
                "greeting": "Hello."
            })
            .to_string();
            for (path, content) in [
                (
                    "character-resources/kokoro/1.0.0/character.json",
                    manifest.as_bytes(),
                ),
                (
                    "character-resources/kokoro/1.0.0/LICENSE.md",
                    b"MIT".as_slice(),
                ),
                (
                    "character-resources/kokoro/1.0.0/CHARACTER.JSON",
                    b"shadow".as_slice(),
                ),
            ] {
                writer.start_file(path, options).unwrap();
                writer.write_all(content).unwrap();
            }
            writer.finish().unwrap();
        }
        fs::write(&archive_path, bytes.into_inner()).unwrap();

        let error =
            stage_character_resources(&archive_path, &temp.path().join("staging")).unwrap_err();

        assert!(error.to_string().contains("duplicate"), "{error}");
    }

    #[test]
    fn resource_promotion_rejects_redirected_catalog_parent() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let catalog = temp.path().join("app/characters");
        let redirected = temp.path().join("redirected-catalog");
        fs::create_dir_all(&catalog).unwrap();
        fs::create_dir_all(&redirected).unwrap();
        let redirected_id = catalog.join("kokoro");
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&redirected, &redirected_id).is_err() {
            return;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&redirected, &redirected_id).unwrap();
        let staged_package = staging.join("kokoro/1.0.0");
        fs::create_dir_all(&staged_package).unwrap();
        fs::write(staged_package.join("character.json"), b"{}").unwrap();
        let staged = StagedCharacterResources {
            packages: vec![("kokoro".to_string(), "1.0.0".to_string())],
            instance_ids: Vec::new(),
        };

        let error = match promote_staged_resources(
            &staging,
            &catalog,
            &staged,
            ConflictStrategy::Overwrite,
        ) {
            Ok(_) => panic!("redirected catalog parent must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("restore parent"), "{error}");
        assert!(!redirected.join("1.0.0").exists());
        assert!(staged_package.exists());
    }

    #[test]
    fn resource_promotion_rejects_redirected_instance_resource_parent() {
        let temp = tempfile::tempdir().unwrap();
        let staging = temp.path().join("staging");
        let catalog = temp.path().join("app/characters");
        let app_data = catalog.parent().unwrap();
        let redirected = temp.path().join("redirected-instances");
        fs::create_dir_all(app_data).unwrap();
        fs::create_dir_all(&redirected).unwrap();
        let resource_root = app_data.join("character-instance-resources");
        #[cfg(windows)]
        if std::os::windows::fs::symlink_dir(&redirected, &resource_root).is_err() {
            return;
        }
        #[cfg(unix)]
        std::os::unix::fs::symlink(&redirected, &resource_root).unwrap();
        let staged_avatar = staging.join(".instances/instance/avatar.png");
        fs::create_dir_all(staged_avatar.parent().unwrap()).unwrap();
        fs::write(&staged_avatar, b"new").unwrap();
        let staged = StagedCharacterResources {
            packages: Vec::new(),
            instance_ids: vec!["instance".to_string()],
        };

        let error = match promote_staged_resources(
            &staging,
            &catalog,
            &staged,
            ConflictStrategy::Overwrite,
        ) {
            Ok(_) => panic!("redirected instance resource parent must be rejected"),
            Err(error) => error,
        };

        assert!(error.to_string().contains("restore parent"), "{error}");
        assert!(!redirected.join("instance").exists());
        assert!(staged_avatar.exists());
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
    fn config_replacement_preserves_local_provider_secrets() {
        let temp = tempfile::tempdir().unwrap();
        let target = temp.path().join("llm_config.json");
        fs::write(
            &target,
            r#"{"providers":[{"id":"local","api_key":"keep-this-secret","extra":{"token":"keep-this-token"}}]}"#,
        )
        .unwrap();
        let configs = vec![(
            "llm_config.json".to_string(),
            r#"{"providers":[{"id":"local","extra":{}}]}"#.to_string(),
        )];

        let mut guard = replace_configs_atomically(temp.path(), &configs).unwrap();
        let restored = fs::read_to_string(&target).unwrap();
        guard.disarm();

        assert!(restored.contains("keep-this-secret"));
        assert!(restored.contains("keep-this-token"));
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
            character_merges: Vec::new(),
            ignored_characters: Vec::new(),
        };
        assert!(!should_stage_character_resources(&options, &inspection));
        options.import_database = true;
        assert!(should_stage_character_resources(&options, &inspection));
    }

    #[test]
    fn config_only_import_does_not_restore_current_conversation_pointer() {
        assert!(!should_import_config(
            "current_conversation_id.json",
            false,
            false
        ));
        assert!(!should_import_config(
            "current_conversation_id.json",
            true,
            false
        ));
        assert!(should_import_config(
            "current_conversation_id.json",
            true,
            true
        ));
        assert!(should_import_config(
            "memory_system_config.json",
            false,
            false
        ));
    }

    #[test]
    fn backup_sanitizer_preserves_non_secret_mcp_env_values() {
        let sanitized = sanitize_backup_config(
            r#"[{"name":"server","env":{"LOG_LEVEL":"debug","OPENAI_API_KEY":"secret"}}]"#,
            "mcp_servers.json",
        )
        .unwrap();
        let value: serde_json::Value = serde_json::from_str(&sanitized).unwrap();
        let env = value[0]["env"].as_object().unwrap();

        assert_eq!(
            env.get("LOG_LEVEL").and_then(|value| value.as_str()),
            Some("debug")
        );
        assert!(!env.contains_key("OPENAI_API_KEY"));
    }

    #[tokio::test]
    async fn overwrite_import_replaces_stale_committed_runtime() {
        use crate::characters::activation::{BackendRuntimeSnapshot, CommittedCharacterRuntime};

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("CREATE TABLE characters (id TEXT PRIMARY KEY, name TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE conversations (id TEXT PRIMARY KEY, character_id TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO characters (id, name) VALUES ('import-character', 'Imported')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, character_id) VALUES ('import-conversation', 'import-character')",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "CREATE TABLE character_activation_runtime (singleton INTEGER PRIMARY KEY CHECK(singleton = 1), revision INTEGER NOT NULL, runtime_json TEXT NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        let stale = CommittedCharacterRuntime {
            revision: 1,
            runtime: BackendRuntimeSnapshot {
                character_id: "stale-character".to_string(),
                current_conversation_id: Some("stale-conversation".to_string()),
                ..Default::default()
            },
            target_conversation_id: "stale-conversation".to_string(),
        };
        sqlx::query(
            "INSERT INTO character_activation_runtime (singleton, revision, runtime_json) VALUES (1, ?, ?)",
        )
        .bind(stale.revision as i64)
        .bind(serde_json::to_string(&stale).unwrap())
        .execute(&mut *connection)
        .await
        .unwrap();

        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.character_activation_runtime (singleton INTEGER PRIMARY KEY, revision INTEGER NOT NULL, runtime_json TEXT NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        let imported = CommittedCharacterRuntime {
            revision: 9,
            runtime: BackendRuntimeSnapshot {
                character_id: "import-character".to_string(),
                current_conversation_id: Some("import-conversation".to_string()),
                ..Default::default()
            },
            target_conversation_id: "import-conversation".to_string(),
        };
        sqlx::query(
            "INSERT INTO import_db.character_activation_runtime (singleton, revision, runtime_json) VALUES (1, ?, ?)",
        )
        .bind(imported.revision as i64)
        .bind(serde_json::to_string(&imported).unwrap())
        .execute(&mut *connection)
        .await
        .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        restore_committed_runtime(&mut transaction, true)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        let restored: String = sqlx::query_scalar(
            "SELECT runtime_json FROM character_activation_runtime WHERE singleton = 1",
        )
        .fetch_one(&mut *connection)
        .await
        .unwrap();
        let restored: CommittedCharacterRuntime = serde_json::from_str(&restored).unwrap();
        assert_eq!(restored.revision, imported.revision);
        assert_eq!(restored.runtime.character_id, "import-character");
        assert_eq!(
            restored.runtime.current_conversation_id.as_deref(),
            Some("import-conversation")
        );
    }

    #[tokio::test]
    async fn overwrite_import_discards_runtime_revision_that_does_not_fit_sqlite() {
        use crate::characters::activation::{BackendRuntimeSnapshot, CommittedCharacterRuntime};

        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("CREATE TABLE characters (id TEXT PRIMARY KEY, name TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE conversations (id TEXT PRIMARY KEY, character_id TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO characters (id, name) VALUES ('import-character', 'Imported')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, character_id) VALUES ('import-conversation', 'import-character')",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.character_activation_runtime (singleton INTEGER PRIMARY KEY, revision INTEGER NOT NULL, runtime_json TEXT NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        let imported = CommittedCharacterRuntime {
            revision: i64::MAX as u64 + 1,
            runtime: BackendRuntimeSnapshot {
                character_id: "import-character".to_string(),
                current_conversation_id: Some("import-conversation".to_string()),
                ..Default::default()
            },
            target_conversation_id: "import-conversation".to_string(),
        };
        sqlx::query(
            "INSERT INTO import_db.character_activation_runtime (singleton, revision, runtime_json) VALUES (1, -1, ?)",
        )
        .bind(serde_json::to_string(&imported).unwrap())
        .execute(&mut *connection)
        .await
        .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        restore_committed_runtime(&mut transaction, true)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM character_activation_runtime")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    /// Mirrors the live conversation schema for the skip-scope tests.
    async fn create_conversation_tables(connection: &mut SqliteConnection) {
        sqlx::query("CREATE TABLE conversations (id TEXT PRIMARY KEY, character_id TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE conversation_messages (\
                id INTEGER PRIMARY KEY, conversation_id TEXT NOT NULL, role TEXT NOT NULL, \
                content TEXT NOT NULL, metadata TEXT, created_at TEXT NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
    }

    async fn create_import_conversation_tables(connection: &mut SqliteConnection) {
        sqlx::query("CREATE TABLE import_db.conversations (id TEXT PRIMARY KEY, character_id TEXT NOT NULL)")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.conversation_messages (\
                id INTEGER PRIMARY KEY, conversation_id TEXT NOT NULL, role TEXT NOT NULL, \
                content TEXT NOT NULL, metadata TEXT, created_at TEXT NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
    }

    #[tokio::test]
    async fn skip_import_never_attaches_messages_to_a_local_conversation() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        create_conversation_tables(&mut connection).await;
        sqlx::query("INSERT INTO conversations (id, character_id) VALUES ('shared-conversation', 'local-character')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO conversation_messages (id, conversation_id, role, content, created_at) VALUES (1, 'shared-conversation', 'user', 'local one', '1')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        create_import_conversation_tables(&mut connection).await;
        sqlx::query("INSERT INTO import_db.conversations (id, character_id) VALUES ('shared-conversation', 'import-character')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("INSERT INTO import_db.conversation_messages (id, conversation_id, role, content, created_at) VALUES (99, 'shared-conversation', 'user', 'imported', '1')")
            .execute(&mut *connection)
            .await
            .unwrap();

        let new_conversations = prepare_skip_conversation_scope(&mut connection)
            .await
            .unwrap();
        assert_eq!(new_conversations, 0);
        sqlx::query("INSERT OR IGNORE INTO conversations (id, character_id) SELECT id, character_id FROM import_db.conversations")
            .execute(&mut *connection)
            .await
            .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        let inserted = insert_skip_conversation_messages(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(
            inserted, 0,
            "a pre-existing local conversation keeps its own history"
        );
        let contents: Vec<String> = sqlx::query_scalar(
            "SELECT content FROM conversation_messages WHERE conversation_id = 'shared-conversation' ORDER BY id",
        )
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        assert_eq!(contents, vec!["local one"]);
    }

    #[tokio::test]
    async fn skip_import_ignores_orphan_messages_that_reference_local_conversations() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        create_conversation_tables(&mut connection).await;
        sqlx::query("INSERT INTO conversations (id, character_id) VALUES ('shared-conversation', 'local-character')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        create_import_conversation_tables(&mut connection).await;
        sqlx::query("INSERT INTO import_db.conversation_messages (id, conversation_id, role, content, created_at) VALUES (99, 'shared-conversation', 'user', 'orphan', '1')")
            .execute(&mut *connection)
            .await
            .unwrap();

        let new_conversations = prepare_skip_conversation_scope(&mut connection)
            .await
            .unwrap();
        assert_eq!(new_conversations, 0);

        let mut transaction = connection.begin().await.unwrap();
        let inserted = insert_skip_conversation_messages(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(inserted, 0);
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM conversation_messages")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn skip_import_ignores_unsupported_import_message_shapes() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        create_conversation_tables(&mut connection).await;
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE VIEW import_db.conversation_messages AS SELECT 99 AS id, 'shared-conversation' AS conversation_id",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        let inserted = insert_skip_conversation_messages(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(inserted, 0);
    }

    #[tokio::test]
    async fn skip_import_moves_new_conversation_history_above_local_message_ids() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        create_conversation_tables(&mut connection).await;
        sqlx::query(
            "INSERT INTO conversations (id, character_id) VALUES ('shared-conversation', 'alice')",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        for (id, content) in [(1_i64, "local one"), (2, "local two")] {
            sqlx::query("INSERT INTO conversation_messages (id, conversation_id, role, content, created_at) VALUES (?, 'shared-conversation', 'user', ?, '1')")
                .bind(id)
                .bind(content)
                .execute(&mut *connection)
                .await
                .unwrap();
        }
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        create_import_conversation_tables(&mut connection).await;
        sqlx::query("INSERT INTO import_db.conversations (id, character_id) VALUES ('shared-conversation', 'alice'), ('new-conversation', 'bob')")
            .execute(&mut *connection)
            .await
            .unwrap();
        for (id, conversation, content) in [
            (1_i64, "shared-conversation", "stale copy one"),
            (2, "shared-conversation", "stale copy two"),
            (3, "new-conversation", "new one"),
            (4, "new-conversation", "new two"),
        ] {
            sqlx::query("INSERT INTO import_db.conversation_messages (id, conversation_id, role, content, created_at) VALUES (?, ?, 'user', ?, '1')")
                .bind(id)
                .bind(conversation)
                .bind(content)
                .execute(&mut *connection)
                .await
                .unwrap();
        }

        let new_conversations = prepare_skip_conversation_scope(&mut connection)
            .await
            .unwrap();
        assert_eq!(new_conversations, 1);
        sqlx::query("INSERT OR IGNORE INTO conversations (id, character_id) SELECT id, character_id FROM import_db.conversations")
            .execute(&mut *connection)
            .await
            .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        let inserted = insert_skip_conversation_messages(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(
            inserted, 2,
            "only the new conversation contributes messages"
        );
        let local: Vec<String> = sqlx::query_scalar(
            "SELECT content FROM conversation_messages WHERE conversation_id = 'shared-conversation' ORDER BY id",
        )
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        assert_eq!(local, vec!["local one", "local two"]);

        let moved: Vec<(i64, String)> = sqlx::query_as(
            "SELECT id, content FROM conversation_messages WHERE conversation_id = 'new-conversation' ORDER BY id",
        )
        .fetch_all(&mut *connection)
        .await
        .unwrap();
        assert_eq!(
            moved,
            vec![(5_i64, "new one".to_string()), (6, "new two".to_string())],
            "imported ids must move above every local message id"
        );
    }

    #[tokio::test]
    async fn overwrite_import_clears_memory_relations_the_backup_does_not_carry() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query(
            "INSERT INTO memories (id, content, embedding, created_at, updated_at, character_id) \
             VALUES (1, 'local memory', X'', 1, 1, 'alice')",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memory_evidence (memory_id, character_id, source_kind, created_at) \
             VALUES (1, 'alice', 'chat', 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memory_dream_proposals (character_id, proposal_type, status, title, created_at, updated_at) \
             VALUES ('alice', 'merge', 'pending', 'stale proposal', 1, 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        // An old backup: no dream tables at all.
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();

        let mut debug_log = Vec::new();
        let mut transaction = connection.begin().await.unwrap();
        restore_memory_aux_tables_overwrite(&mut transaction, &mut debug_log)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        for table in MEMORY_REFERENCING_TABLES {
            let remaining: i64 = sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table}"))
                .fetch_one(&mut *connection)
                .await
                .unwrap();
            assert_eq!(
                remaining, 0,
                "{table} must not keep rows that point at replaced memory ids"
            );
        }
        assert!(debug_log
            .iter()
            .any(|line| line.contains("memory_evidence")));
    }

    #[tokio::test]
    async fn skip_import_keeps_local_rows_and_drops_only_stale_relations() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        for (id, content) in [(1_i64, "unchanged memory"), (2, "local edit")] {
            sqlx::query(
                "INSERT INTO memories (id, content, embedding, created_at, updated_at, importance, character_id, tier, canonical_hash) \
                 VALUES (?, ?, X'', 1, 1, 0.5, 'alice', 'ephemeral', ?)",
            )
            .bind(id)
            .bind(content)
            .bind(content)
            .execute(&mut *connection)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO memory_operations (id, character_id, operation_type, actor, memory_id, created_at) \
             VALUES (10, 'alice', 'insert_active', 'pipeline', 1, 1), (11, 'alice', 'insert_active', 'pipeline', 2, 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE import_db.memories AS SELECT * FROM memories WHERE 0")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.memory_operations AS SELECT * FROM memory_operations WHERE 0",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        for (id, content, supersedes) in [
            (1_i64, "unchanged memory", None),
            (2, "backup version of the edited memory", None),
            (3, "brand new memory", Some("[2]")),
        ] {
            sqlx::query(
                "INSERT INTO import_db.memories (id, content, embedding, created_at, updated_at, importance, \
                 character_id, tier, canonical_hash, supersedes, memory_type, status, confidence, \
                 first_seen_at, last_seen_at, evidence_count, source_kind, source_refs) \
                 VALUES (?, ?, X'', 1, 1, 0.5, 'alice', 'ephemeral', ?, ?, 'fact', 'active', 0.6, 1, 1, 1, 'extractor', '[]')",
            )
            .bind(id)
            .bind(content)
            .bind(content)
            .bind(supersedes)
            .execute(&mut *connection)
            .await
            .unwrap();
        }
        sqlx::query(
            "INSERT INTO import_db.memory_operations (id, character_id, operation_type, actor, memory_id, created_at) \
             VALUES (10, 'alice', 'insert_active', 'pipeline', 1, 1), \
                    (12, 'alice', 'insert_active', 'pipeline', 2, 1), \
                    (13, 'alice', 'insert_active', 'pipeline', 3, 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        let conflicts = resolve_skip_memory_conflicts(&mut connection)
            .await
            .unwrap();
        assert_eq!(
            conflicts.conflicting_ids, 1,
            "only the memory that really differs is a conflict"
        );
        assert_eq!(
            conflicts.dropped_relations, 2,
            "one audit row and one supersedes link point at the conflicting memory"
        );

        let mut transaction = connection.begin().await.unwrap();
        let inserted = sqlx::query(MEMORY_INSERT_SKIP_SQL)
            .execute(&mut *transaction)
            .await
            .unwrap()
            .rows_affected();
        insert_skip_memory_relations(&mut transaction)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(inserted, 1, "only the brand new memory is inserted");

        let local_edit: String = sqlx::query_scalar("SELECT content FROM memories WHERE id = 2")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(local_edit, "local edit");

        let supersedes: Option<String> =
            sqlx::query_scalar("SELECT supersedes FROM memories WHERE id = 3")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        assert_eq!(
            supersedes.as_deref(),
            Some("[]"),
            "a link into a conflicting local memory must be cleared"
        );

        let operations: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM memory_operations ORDER BY id")
                .fetch_all(&mut *connection)
                .await
                .unwrap();
        assert_eq!(
            operations,
            vec![10, 11, 13],
            "the audit row for the conflicting memory is dropped, the new one is kept"
        );
    }

    #[tokio::test]
    async fn import_purges_memories_without_a_local_character() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("INSERT INTO characters (id, name) VALUES ('alice', 'Alice'), ('bob', 'Bob')")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO memories (id, content, embedding, created_at, updated_at, character_id) \
             VALUES (1, 'owned', X'', 1, 1, 'alice'), \
                    (2, 'legacy default', X'', 1, 1, 'default'), \
                    (3, 'deleted owner', X'', 1, 1, 'ghost')",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memory_evidence (memory_id, character_id, source_kind, created_at) \
             VALUES (1, 'alice', 'chat', 1), (2, 'default', 'chat', 1), (3, 'ghost', 'chat', 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memory_dream_jobs (character_id, phase, status, trigger, started_at) \
             VALUES ('ghost', 'dream', 'completed', 'manual', 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        let purged = purge_orphaned_memories(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        assert_eq!(purged, 2);
        let contents: Vec<String> = sqlx::query_scalar("SELECT content FROM memories")
            .fetch_all(&mut *connection)
            .await
            .unwrap();
        assert_eq!(contents, vec!["owned"]);

        let evidence: Vec<i64> = sqlx::query_scalar("SELECT memory_id FROM memory_evidence")
            .fetch_all(&mut *connection)
            .await
            .unwrap();
        assert_eq!(
            evidence,
            vec![1],
            "relations of purged memories must go too"
        );

        let jobs: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memory_dream_jobs")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(
            jobs, 0,
            "per-character rows of a missing character must go too"
        );
    }

    /// Resolver used by the cross-machine fixtures: no character packages exist,
    /// which is exactly the "restore onto a clean machine" situation.
    struct NoPackages;
    impl CharacterPackageResolver for NoPackages {
        fn resolve_exact(
            &self,
            _template_id: &str,
            _template_version: &str,
        ) -> Result<Option<ResolvedCharacterPackage>, String> {
            Ok(None)
        }
    }

    /// Two databases as they look on two different machines: the same character
    /// display name, but a different instance id on each side.
    async fn cross_machine_pools() -> (tempfile::TempDir, SqlitePool, SqlitePool) {
        let temp = tempfile::tempdir().unwrap();
        let live_url = format!(
            "sqlite://{}",
            temp.path()
                .join("live.db")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let backup_url = format!(
            "sqlite://{}",
            temp.path()
                .join("backup.db")
                .to_string_lossy()
                .replace('\\', "/")
        );
        let live = SqliteConnectOptions::from_str(&live_url)
            .unwrap()
            .create_if_missing(true);
        let live = SqlitePool::connect_with(live).await.unwrap();
        let backup = SqliteConnectOptions::from_str(&backup_url)
            .unwrap()
            .create_if_missing(true);
        let backup = SqlitePool::connect_with(backup).await.unwrap();
        sqlx::migrate!("./migrations").run(&live).await.unwrap();
        sqlx::migrate!("./migrations").run(&backup).await.unwrap();

        // This machine.
        sqlx::query("INSERT INTO characters (id, name) VALUES ('local-pc', 'Kokoro')")
            .execute(&live)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, created_at, updated_at) \
             VALUES ('conv-local', 'local-pc', 'Local chat', 'now', 'now')",
        )
        .execute(&live)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, created_at) \
             VALUES ('conv-local', 'user', 'local message', 'now')",
        )
        .execute(&live)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, updated_at, character_id) \
             VALUES ('local memory', X'', 1, 1, 'local-pc')",
        )
        .execute(&live)
        .await
        .unwrap();

        // The other machine: same display name, different instance id.
        sqlx::query("INSERT INTO characters (id, name) VALUES ('remote-pc', 'Kokoro')")
            .execute(&backup)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, created_at, updated_at) \
             VALUES ('conv-remote', 'remote-pc', 'Remote chat', 'now', 'now')",
        )
        .execute(&backup)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, created_at) \
             VALUES ('conv-remote', 'user', 'remote message', 'now')",
        )
        .execute(&backup)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, updated_at, character_id) \
             VALUES ('remote memory', X'', 1, 1, 'remote-pc')",
        )
        .execute(&backup)
        .await
        .unwrap();

        (temp, live, backup)
    }

    /// Result of replaying the overwrite branch of `import_data`.
    struct OverwriteOutcome {
        conn: sqlx::pool::PoolConnection<sqlx::Sqlite>,
        purged: i64,
        merged: i64,
        ignored: i64,
    }

    /// Replays the overwrite branch of `import_data` for the fixture above. The
    /// statement order mirrors production: ignore, merge, copy, characters, purge.
    async fn run_cross_machine_overwrite(
        live: &SqlitePool,
        backup: &SqlitePool,
        backup_path: &Path,
        merges: &[CharacterMerge],
        ignored: &[&str],
    ) -> OverwriteOutcome {
        let mut prepared = prepare_character_rows(backup, &NoPackages).await.unwrap();

        let mut conn = live.acquire().await.unwrap();
        sqlx::query("ATTACH DATABASE ? AS import_db")
            .bind(backup_path.to_string_lossy().replace('\\', "/"))
            .execute(&mut *conn)
            .await
            .unwrap();

        let mut transaction = conn.begin().await.unwrap();
        let ignored_ids: HashSet<String> = ignored.iter().map(|id| id.to_string()).collect();
        for ignored_id in &ignored_ids {
            remove_imported_character(&mut transaction, ignored_id)
                .await
                .unwrap();
        }
        let ignored_characters = prepared
            .iter()
            .filter(|row| ignored_ids.contains(&row.id))
            .count() as i64;
        prepared.retain(|row| !ignored_ids.contains(&row.id));

        apply_import_character_merges(&mut transaction, merges)
            .await
            .unwrap();
        let merged_ids: HashSet<&str> = merges
            .iter()
            .map(|merge| merge.imported_id.as_str())
            .collect();
        let merged_characters = prepared
            .iter()
            .filter(|row| merged_ids.contains(row.id.as_str()))
            .count() as i64;
        prepared.retain(|row| !merged_ids.contains(row.id.as_str()));

        for statement in [
            "DELETE FROM conversation_messages",
            "DELETE FROM conversations",
            "DELETE FROM memories",
        ] {
            sqlx::query(statement)
                .execute(&mut *transaction)
                .await
                .unwrap();
        }
        sqlx::query(
            "INSERT INTO memories (id, content, embedding, created_at, updated_at, importance, character_id, tier, \
             consolidated_from, memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, \
             evidence_count, source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at) \
             SELECT id, content, embedding, created_at, updated_at, importance, character_id, tier, \
             consolidated_from, memory_type, entity_key, status, confidence, first_seen_at, last_seen_at, \
             evidence_count, source_kind, source_refs, supersedes, canonical_hash, last_dreamed_at FROM import_db.memories",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, topic, pinned_state, created_at, updated_at) \
             SELECT id, character_id, title, topic, pinned_state, created_at, updated_at FROM import_db.conversations",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversation_messages (id, conversation_id, role, content, metadata, created_at) \
             SELECT id, conversation_id, role, content, metadata, created_at FROM import_db.conversation_messages",
        )
        .execute(&mut *transaction)
        .await
        .unwrap();
        apply_character_rows(&mut transaction, prepared, "overwrite")
            .await
            .unwrap();
        let purged = purge_orphaned_memories(&mut transaction).await.unwrap();
        transaction.commit().await.unwrap();

        OverwriteOutcome {
            conn,
            purged,
            merged: merged_characters,
            ignored: ignored_characters,
        }
    }

    #[tokio::test]
    async fn cross_machine_overwrite_restore_keeps_the_backup_character_identity() {
        let (temp, live, backup) = cross_machine_pools().await;
        let backup_path = temp.path().join("backup.db");

        let OverwriteOutcome {
            mut conn,
            purged,
            merged,
            ignored,
        } = run_cross_machine_overwrite(&live, &backup, &backup_path, &[], &[]).await;

        assert_eq!(
            purged, 0,
            "memories of the imported character must survive the orphan purge"
        );
        assert_eq!(merged, 0);
        assert_eq!(ignored, 0);

        let mut names: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM characters ORDER BY id")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        names.sort();
        assert_eq!(
            names,
            vec![
                ("local-pc".to_string(), "Kokoro".to_string()),
                ("remote-pc".to_string(), "Kokoro".to_string()),
            ],
            "without a merge the import never deduplicates by display name"
        );

        let memories: Vec<(String, String)> =
            sqlx::query_as("SELECT character_id, content FROM memories")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            memories,
            vec![("remote-pc".to_string(), "remote memory".to_string())]
        );

        let conversations: Vec<(String, String)> =
            sqlx::query_as("SELECT id, character_id FROM conversations")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            conversations,
            vec![("conv-remote".to_string(), "remote-pc".to_string())]
        );
    }

    #[tokio::test]
    async fn cross_machine_overwrite_restore_can_merge_into_a_local_character() {
        let (temp, live, backup) = cross_machine_pools().await;
        let backup_path = temp.path().join("backup.db");

        let OverwriteOutcome {
            mut conn,
            purged,
            merged,
            ..
        } = run_cross_machine_overwrite(
            &live,
            &backup,
            &backup_path,
            &[CharacterMerge {
                imported_id: "remote-pc".to_string(),
                target_id: "local-pc".to_string(),
            }],
            &[],
        )
        .await;

        assert_eq!(
            merged, 1,
            "the merged character is not imported as a new one"
        );
        assert_eq!(purged, 0);

        let names: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM characters ORDER BY id")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            names,
            vec![("local-pc".to_string(), "Kokoro".to_string())],
            "a merge leaves exactly one character behind"
        );

        let memories: Vec<(String, String)> =
            sqlx::query_as("SELECT character_id, content FROM memories")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            memories,
            vec![("local-pc".to_string(), "remote memory".to_string())],
            "imported memories follow the local character"
        );

        let conversations: Vec<(String, String)> =
            sqlx::query_as("SELECT id, character_id FROM conversations")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            conversations,
            vec![("conv-remote".to_string(), "local-pc".to_string())],
            "imported conversations follow the local character"
        );
    }

    #[tokio::test]
    async fn cross_machine_overwrite_restore_can_ignore_a_backup_character() {
        let (temp, live, backup) = cross_machine_pools().await;
        let backup_path = temp.path().join("backup.db");

        // A second character in the backup that the user does not want at all.
        sqlx::query("INSERT INTO characters (id, name) VALUES ('remote-archive', 'Archive')")
            .execute(&backup)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO conversations (id, character_id, title, created_at, updated_at) \
             VALUES ('conv-archive', 'remote-archive', 'Archive chat', 'now', 'now')",
        )
        .execute(&backup)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO conversation_messages (conversation_id, role, content, created_at) \
             VALUES ('conv-archive', 'user', 'archived message', 'now')",
        )
        .execute(&backup)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO memories (content, embedding, created_at, updated_at, character_id) \
             VALUES ('archived memory', X'', 1, 1, 'remote-archive')",
        )
        .execute(&backup)
        .await
        .unwrap();

        let OverwriteOutcome {
            mut conn,
            purged,
            merged,
            ignored,
        } = run_cross_machine_overwrite(&live, &backup, &backup_path, &[], &["remote-archive"])
            .await;

        assert_eq!(ignored, 1, "the ignored character is reported");
        assert_eq!(merged, 0);
        assert_eq!(purged, 0, "nothing is left behind for the orphan purge");

        let mut names: Vec<(String, String)> =
            sqlx::query_as("SELECT id, name FROM characters ORDER BY id")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        names.sort();
        assert_eq!(
            names,
            vec![
                ("local-pc".to_string(), "Kokoro".to_string()),
                ("remote-pc".to_string(), "Kokoro".to_string()),
            ],
            "the ignored character is not imported"
        );

        let memories: Vec<(String, String)> =
            sqlx::query_as("SELECT character_id, content FROM memories")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            memories,
            vec![("remote-pc".to_string(), "remote memory".to_string())],
            "memories of the ignored character are not copied"
        );

        let conversations: Vec<(String, String)> =
            sqlx::query_as("SELECT id, character_id FROM conversations ORDER BY id")
                .fetch_all(&mut *conn)
                .await
                .unwrap();
        assert_eq!(
            conversations,
            vec![("conv-remote".to_string(), "remote-pc".to_string())]
        );

        let messages: Vec<String> = sqlx::query_scalar("SELECT content FROM conversation_messages")
            .fetch_all(&mut *conn)
            .await
            .unwrap();
        assert_eq!(
            messages,
            vec!["remote message"],
            "messages of the ignored character are not copied"
        );
    }

    #[tokio::test]
    async fn preview_lists_backup_characters_with_their_data_counts() {
        let (temp, live, backup) = cross_machine_pools().await;

        let summaries = read_backup_characters(&temp.path().join("backup.db")).await;
        assert_eq!(
            summaries.len(),
            1,
            "the preview must expose the backup characters"
        );
        assert_eq!(summaries[0].id, "remote-pc");
        assert_eq!(summaries[0].name, "Kokoro");
        assert_eq!(summaries[0].memory_count, 1);
        assert_eq!(summaries[0].conversation_count, 1);

        // A backup from before characters moved into SQLite has no table at all.
        sqlx::query("DROP TABLE characters")
            .execute(&backup)
            .await
            .unwrap();
        assert!(read_backup_characters(&temp.path().join("backup.db"))
            .await
            .is_empty());

        drop(live);
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

    #[tokio::test]
    async fn consistent_database_snapshot_preserves_wal_commits() {
        let temp = tempfile::tempdir().unwrap();
        let source = temp.path().join("source.db");
        let url = format!("sqlite://{}", source.to_string_lossy().replace('\\', "/"));
        let options = SqliteConnectOptions::from_str(&url)
            .unwrap()
            .create_if_missing(true);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::query("PRAGMA journal_mode = WAL")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("CREATE TABLE records (id INTEGER PRIMARY KEY, value TEXT NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO records (id, value) VALUES (1, 'committed')")
            .execute(&pool)
            .await
            .unwrap();

        let snapshot_bytes = create_consistent_database_snapshot(&source)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(snapshot_bytes.stats.memories, 0);
        let bytes = snapshot_bytes.bytes;
        pool.close().await;
        let snapshot = temp.path().join("snapshot.db");
        fs::write(&snapshot, bytes).unwrap();
        let snapshot_url = format!("sqlite://{}", snapshot.to_string_lossy().replace('\\', "/"));
        let snapshot_pool = SqlitePool::connect(&snapshot_url).await.unwrap();
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM records")
            .fetch_one(&snapshot_pool)
            .await
            .unwrap();
        snapshot_pool.close().await;

        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn optional_backup_tables_restore_session_summaries() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.session_summaries (id INTEGER PRIMARY KEY, character_id TEXT NOT NULL, summary TEXT NOT NULL, created_at INTEGER NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO import_db.session_summaries (id, character_id, summary, created_at) VALUES (1, 'character-a', 'restored summary', 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        restore_optional_backup_tables(&mut transaction, false)
            .await
            .unwrap();
        transaction.commit().await.unwrap();
        let summary: String =
            sqlx::query_scalar("SELECT summary FROM session_summaries WHERE id = 1")
                .fetch_one(&mut *connection)
                .await
                .unwrap();

        assert_eq!(summary, "restored summary");
    }

    #[tokio::test]
    async fn optional_backup_overwrite_keeps_local_rows_when_import_table_is_missing() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query(
            "INSERT INTO session_summaries (character_id, summary, created_at) VALUES ('local', 'keep me', 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        restore_optional_backup_tables(&mut transaction, true)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        let summary: String = sqlx::query_scalar(
            "SELECT summary FROM session_summaries WHERE character_id = 'local'",
        )
        .fetch_one(&mut *connection)
        .await
        .unwrap();
        assert_eq!(summary, "keep me");
    }

    #[tokio::test]
    async fn target_character_remap_ignores_legacy_emotion_snapshots() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.emotion_snapshots (character_id TEXT PRIMARY KEY, emotion TEXT NOT NULL, mood REAL NOT NULL, accumulated_inertia REAL NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO import_db.emotion_snapshots (character_id, emotion, mood, accumulated_inertia, updated_at) VALUES ('character-a', 'calm', 0.2, 0.1, 1), ('character-b', 'happy', 0.8, 0.3, 2)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        let remapped = apply_import_character_merges(
            &mut connection,
            &[CharacterMerge {
                imported_id: "character-a".to_string(),
                target_id: "target-character".to_string(),
            }],
        )
        .await
        .unwrap();

        assert!(!remapped
            .iter()
            .any(|(table, _)| table.starts_with("emotion_snapshots")));
        let snapshot_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM import_db.emotion_snapshots")
                .fetch_one(&mut *connection)
                .await
                .unwrap();
        assert_eq!(snapshot_count, 2);
    }

    #[tokio::test]
    async fn optional_backup_restore_ignores_legacy_emotion_snapshots() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let mut connection = pool.acquire().await.unwrap();
        sqlx::query("ATTACH DATABASE ':memory:' AS import_db")
            .execute(&mut *connection)
            .await
            .unwrap();
        sqlx::query(
            "CREATE TABLE import_db.emotion_snapshots (character_id TEXT PRIMARY KEY, emotion TEXT NOT NULL, mood REAL NOT NULL, accumulated_inertia REAL NOT NULL, updated_at INTEGER NOT NULL)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO import_db.emotion_snapshots (character_id, emotion, mood, accumulated_inertia, updated_at) VALUES ('legacy-character', 'calm', 0.2, 0.1, 1)",
        )
        .execute(&mut *connection)
        .await
        .unwrap();

        let mut transaction = connection.begin().await.unwrap();
        restore_optional_backup_tables(&mut transaction, false)
            .await
            .unwrap();
        transaction.commit().await.unwrap();

        let snapshot_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM emotion_snapshots")
            .fetch_one(&mut *connection)
            .await
            .unwrap();
        assert_eq!(snapshot_count, 0);
    }
}
