// pattern: Imperative Shell

use super::manifest::{CharacterTemplateManifest, ManifestError};
use super::{validate_package_content, PackageContentEntry, PackageContentError};
use semver::Version;
use std::fs;
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;
use zip::result::ZipError;

#[cfg(windows)]
use std::os::windows::fs::MetadataExt;

#[derive(Clone, Debug, PartialEq)]
pub struct CatalogEntry {
    pub manifest: CharacterTemplateManifest,
    pub package_dir: PathBuf,
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("character catalog I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("invalid character archive: {0}")]
    Zip(#[from] ZipError),
    #[error(transparent)]
    Content(#[from] PackageContentError),
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    #[error("character package layout does not match manifest `{id}` version `{version}`")]
    Layout { id: String, version: String },
    #[error("declared asset is missing: {0}")]
    MissingDeclaredAsset(String),
    #[error("declared asset must be a regular non-symlink file: {0}")]
    InvalidDeclaredAsset(String),
    #[error("character archive contains duplicate path `{0}`")]
    DuplicateArchivePath(String),
}

pub struct CharacterCatalog {
    root: PathBuf,
    engine_version: Version,
}

pub struct StagedPackageRemoval {
    target: PathBuf,
    staging: Option<PathBuf>,
}

impl StagedPackageRemoval {
    pub fn rollback(mut self) -> Result<(), CatalogError> {
        self.rollback_inner()
    }

    pub fn finalize(mut self) -> Result<(), CatalogError> {
        let Some(staging) = self.staging.take() else {
            return Ok(());
        };
        if let Err(error) = remove_non_redirected_directory(&staging) {
            let _ = fs::rename(&staging, &self.target);
            return Err(error.into());
        }
        Ok(())
    }

    fn rollback_inner(&mut self) -> Result<(), CatalogError> {
        let Some(staging) = self.staging.take() else {
            return Ok(());
        };
        let metadata = fs::symlink_metadata(&staging)?;
        if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
            return Err(CatalogError::InvalidDeclaredAsset(
                staging.display().to_string(),
            ));
        }
        fs::rename(staging, &self.target)?;
        Ok(())
    }
}

impl Drop for StagedPackageRemoval {
    fn drop(&mut self) {
        if self.staging.is_some() {
            if let Err(error) = self.rollback_inner() {
                tracing::error!(
                    target: "characters",
                    path = %self.target.display(),
                    "failed to roll back staged package removal: {error}"
                );
            }
        }
    }
}

impl CharacterCatalog {
    pub fn new(root: PathBuf, engine_version: Version) -> Self {
        Self {
            root,
            engine_version,
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Resolve an exact immutable template version from the local catalog.
    /// Invalid path segments simply behave as unavailable versions.
    pub fn find_exact(&self, id: &str, version: &str) -> Option<CatalogEntry> {
        if !is_safe_catalog_id(id) || !is_safe_catalog_version(version) {
            return None;
        }
        let target = self.root.join(id).join(version);
        let target_parent = target.parent()?;
        if ensure_catalog_parent_chain(&self.root, target_parent).is_err()
            || reject_case_folded_sibling(target_parent, "character id").is_err()
            || reject_case_folded_sibling(&target, "character version").is_err()
        {
            return None;
        }
        self.validate_installed_directory(&target).ok()
    }

    /// Return a usable presentation directory, or `None` so callers can use
    /// the built-in Live2D/background fallback when an archived version is
    /// unavailable.
    pub fn presentation_directory(&self, id: &str, version: &str) -> Option<PathBuf> {
        self.find_exact(id, version).map(|entry| entry.package_dir)
    }

    /// Remove only package-owned resources. User instances, conversations,
    /// memories, and settings live outside this directory and are untouched.
    /// The target is staged by rename first, making an interrupted removal
    /// recoverable until the final delete succeeds.
    pub fn remove_package(&self, id: &str, version: &str) -> Result<(), CatalogError> {
        if let Some(staged) = self.stage_package_removal(id, version)? {
            staged.finalize()?;
        }
        Ok(())
    }

    pub fn stage_package_removal(
        &self,
        id: &str,
        version: &str,
    ) -> Result<Option<StagedPackageRemoval>, CatalogError> {
        if !is_safe_catalog_id(id) || !is_safe_catalog_version(version) {
            return Err(CatalogError::Layout {
                id: id.to_string(),
                version: version.to_string(),
            });
        }
        let target = self.root.join(id).join(version);
        ensure_catalog_parent_chain(
            &self.root,
            target.parent().expect("catalog target has parent"),
        )?;
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
            return Err(CatalogError::InvalidDeclaredAsset(
                target.display().to_string(),
            ));
        }
        let staging = self
            .root
            .join(format!(".delete-{id}-{version}-{}", Uuid::new_v4()));
        fs::rename(&target, &staging)?;
        Ok(Some(StagedPackageRemoval {
            target,
            staging: Some(staging),
        }))
    }

    pub fn discover(&self) -> Result<Vec<CatalogEntry>, CatalogError> {
        ensure_catalog_directory(&self.root)?;
        let mut packages = Vec::new();
        for id_entry in fs::read_dir(&self.root)? {
            let id_entry = id_entry?;
            let id_path = id_entry.path();
            let id_metadata = fs::symlink_metadata(&id_path)?;
            if is_filesystem_redirect(&id_metadata)
                || !id_metadata.is_dir()
                || id_entry.file_name().to_string_lossy().starts_with('.')
                || reject_case_folded_sibling(&id_path, "character id").is_err()
            {
                continue;
            }
            for version_entry in fs::read_dir(&id_path)? {
                let version_entry = version_entry?;
                let version_path = version_entry.path();
                let version_metadata = fs::symlink_metadata(&version_path)?;
                if is_filesystem_redirect(&version_metadata)
                    || !version_metadata.is_dir()
                    || version_entry.file_name().to_string_lossy().starts_with('.')
                    || reject_case_folded_sibling(&version_path, "character version").is_err()
                {
                    continue;
                }
                match self.validate_installed_directory(&version_path) {
                    Ok(entry) => packages.push(entry),
                    Err(error) => tracing::warn!(
                        target: "characters",
                        "skipping invalid character package {}: {}",
                        version_entry.path().display(),
                        error
                    ),
                }
            }
        }
        packages.sort_by(|left, right| {
            left.manifest
                .id
                .cmp(&right.manifest.id)
                .then_with(|| left.manifest.version.cmp(&right.manifest.version))
        });
        Ok(packages)
    }

    pub fn install_bundled(&self, bundled_root: &Path) -> Result<Vec<CatalogEntry>, CatalogError> {
        ensure_catalog_directory(&self.root)?;
        ensure_catalog_parent_chain(bundled_root, bundled_root)?;
        let bundled_metadata = match fs::symlink_metadata(bundled_root) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
            Err(error) => return Err(error.into()),
        };
        if is_filesystem_redirect(&bundled_metadata) || !bundled_metadata.is_dir() {
            return Err(CatalogError::InvalidDeclaredAsset(format!(
                "bundled character root is not a regular directory: {}",
                bundled_root.display()
            )));
        }

        let mut installed = Vec::new();
        for package in fs::read_dir(bundled_root)? {
            let package = package?;
            let package_path = package.path();
            let package_metadata = fs::symlink_metadata(&package_path)?;
            if !is_filesystem_redirect(&package_metadata)
                && package_metadata.is_dir()
                && package_path.join("character.json").is_file()
            {
                installed.push(self.install_directory(&package_path)?);
            }
        }
        installed.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(installed)
    }

    pub fn install_zip<R: Read + Seek>(&self, reader: R) -> Result<CatalogEntry, CatalogError> {
        ensure_catalog_directory(&self.root)?;
        let mut archive = zip::ZipArchive::new(reader)?;
        let entries = archive_entries(&mut archive)?;
        validate_package_content(&entries)?;

        let staging = self.staging_path();
        fs::create_dir_all(&staging)?;
        let extraction = (|| -> Result<(), CatalogError> {
            for index in 0..archive.len() {
                let mut source = archive.by_index(index)?;
                let relative = normalized_archive_path(source.name(), source.is_dir());
                let destination = staging.join(relative);
                if source.is_dir() {
                    fs::create_dir_all(destination)?;
                    continue;
                }
                if let Some(parent) = destination.parent() {
                    fs::create_dir_all(parent)?;
                }
                let mut output = fs::File::create(destination)?;
                let copied = io::copy(&mut source, &mut output)?;
                if copied != source.size() {
                    return Err(CatalogError::Io(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "archive entry size changed during extraction",
                    )));
                }
            }
            Ok(())
        })();
        if let Err(error) = extraction {
            remove_non_redirected_directory(&staging).ok();
            return Err(error);
        }

        match self.commit_staging(staging) {
            Ok(entry) => Ok(entry),
            Err(error) => Err(error),
        }
    }

    fn install_directory(&self, source: &Path) -> Result<CatalogEntry, CatalogError> {
        let manifest = self.validate_source_directory(source)?;
        let staging = self.staging_path();
        fs::create_dir_all(&staging)?;
        if let Err(error) = copy_package_directory(source, &staging) {
            remove_non_redirected_directory(&staging).ok();
            return Err(error.into());
        }
        self.commit_validated_staging(staging, manifest)
    }

    fn commit_staging(&self, staging: PathBuf) -> Result<CatalogEntry, CatalogError> {
        let manifest = match self.validate_source_directory(&staging) {
            Ok(manifest) => manifest,
            Err(error) => {
                remove_non_redirected_directory(&staging).ok();
                return Err(error);
            }
        };
        self.commit_validated_staging(staging, manifest)
    }

    fn commit_validated_staging(
        &self,
        staging: PathBuf,
        manifest: CharacterTemplateManifest,
    ) -> Result<CatalogEntry, CatalogError> {
        let target = self.root.join(&manifest.id).join(&manifest.version);
        ensure_catalog_parent_chain(
            &self.root,
            target.parent().expect("catalog target has parent"),
        )?;
        reject_case_folded_sibling(
            target.parent().expect("catalog target has parent"),
            "character id",
        )?;
        reject_case_folded_sibling(&target, "character version")?;
        if let Ok(metadata) = fs::symlink_metadata(&target) {
            if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                return Err(CatalogError::InvalidDeclaredAsset(
                    target.display().to_string(),
                ));
            }
        }
        if let Some(parent) = target.parent() {
            create_directory_chain(parent)?;
        }
        atomic_replace_directory(&staging, &target)?;
        Ok(CatalogEntry {
            manifest,
            package_dir: target,
        })
    }

    fn validate_installed_directory(
        &self,
        package_dir: &Path,
    ) -> Result<CatalogEntry, CatalogError> {
        let manifest = self.validate_source_directory(package_dir)?;
        let expected_id = package_dir
            .parent()
            .and_then(Path::file_name)
            .and_then(|value| value.to_str());
        let expected_version = package_dir.file_name().and_then(|value| value.to_str());
        if expected_id != Some(manifest.id.as_str())
            || expected_version != Some(manifest.version.as_str())
        {
            return Err(CatalogError::Layout {
                id: manifest.id,
                version: manifest.version,
            });
        }
        Ok(CatalogEntry {
            manifest,
            package_dir: package_dir.to_path_buf(),
        })
    }

    fn validate_source_directory(
        &self,
        package_dir: &Path,
    ) -> Result<CharacterTemplateManifest, CatalogError> {
        validate_package_directory(package_dir, &self.engine_version)
    }

    fn staging_path(&self) -> PathBuf {
        self.root.join(format!(".staging-{}", Uuid::new_v4()))
    }
}

fn is_safe_catalog_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
}

fn is_safe_catalog_version(value: &str) -> bool {
    let path = Path::new(value);
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && path.components().count() == 1
        && path.file_name().and_then(|name| name.to_str()) == Some(value)
        && Version::parse(value).is_ok()
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

fn ensure_catalog_parent_chain(root: &Path, target_parent: &Path) -> Result<(), io::Error> {
    let relative = target_parent.strip_prefix(root).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "catalog target escapes managed root: {}",
                target_parent.display()
            ),
        )
    })?;
    if relative
        .components()
        .any(|component| !matches!(component, std::path::Component::Normal(_)))
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "catalog target contains unsafe parent path: {}",
                target_parent.display()
            ),
        ));
    }

    let mut existing = target_parent.to_path_buf();
    loop {
        match fs::symlink_metadata(&existing) {
            Ok(metadata) => {
                if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "catalog parent is not a regular directory: {}",
                            existing.display()
                        ),
                    ));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                existing = existing
                    .parent()
                    .ok_or_else(|| {
                        io::Error::new(
                            io::ErrorKind::InvalidInput,
                            "catalog target has no existing parent",
                        )
                    })?
                    .to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }

    // Check the existing chain too: an app-data root redirected through a
    // junction must not become an implicit write target.
    let mut ancestor = existing;
    loop {
        let metadata = fs::symlink_metadata(&ancestor)?;
        if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "catalog parent is not a regular directory: {}",
                    ancestor.display()
                ),
            ));
        }
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if parent == ancestor {
            break;
        }
        ancestor = parent.to_path_buf();
    }

    Ok(())
}

fn ensure_catalog_directory(root: &Path) -> Result<(), io::Error> {
    ensure_catalog_parent_chain(root, root)?;
    create_directory_chain(root)
}

fn create_directory_chain(path: &Path) -> Result<(), io::Error> {
    let mut missing = Vec::new();
    let mut current = path.to_path_buf();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(metadata) => {
                if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
                    return Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!(
                            "catalog parent is not a regular directory: {}",
                            current.display()
                        ),
                    ));
                }
                break;
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                missing.push(current.clone());
                current = current
                    .parent()
                    .ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidInput, "catalog path has no parent")
                    })?
                    .to_path_buf();
            }
            Err(error) => return Err(error),
        }
    }
    for directory in missing.into_iter().rev() {
        match fs::create_dir(&directory) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
        let metadata = fs::symlink_metadata(&directory)?;
        if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "catalog parent is not a regular directory: {}",
                    directory.display()
                ),
            ));
        }
    }
    Ok(())
}

fn reject_case_folded_sibling(path: &Path, label: &str) -> Result<(), io::Error> {
    let Some(name) = path.file_name().and_then(|value| value.to_str()) else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("invalid {label} path"),
        ));
    };
    let Some(parent) = path.parent() else {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{label} has no parent"),
        ));
    };
    let entries = match fs::read_dir(parent) {
        Ok(entries) => entries,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    for entry in entries {
        let entry = entry?;
        let existing = entry.file_name();
        let existing = existing.to_string_lossy();
        if existing != name && existing.eq_ignore_ascii_case(name) {
            return Err(io::Error::new(
                io::ErrorKind::AlreadyExists,
                format!("case-insensitive {label} collision between `{name}` and `{existing}`"),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_package_directory(
    package_dir: &Path,
    engine_version: &Version,
) -> Result<CharacterTemplateManifest, CatalogError> {
    let entries = directory_entries(package_dir)?;
    validate_package_content(&entries)?;
    let raw = fs::read_to_string(package_dir.join("character.json"))?;
    let manifest = CharacterTemplateManifest::from_json(&raw)?;
    manifest.validate_for_engine(engine_version)?;
    validate_declared_assets(package_dir, &manifest)?;
    Ok(manifest)
}

fn validate_declared_assets(
    package_dir: &Path,
    manifest: &CharacterTemplateManifest,
) -> Result<(), CatalogError> {
    let mut declared = Vec::new();
    if let Some(path) = manifest.avatar.as_deref() {
        declared.push(path);
    }
    if let Some(assets) = manifest.assets.as_ref() {
        declared.extend(assets.live2d_model.as_deref());
        declared.extend(assets.background.as_deref());
        declared.extend(assets.cue_profile.as_deref());
    }

    for relative in declared {
        let path = package_dir.join(relative);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Err(CatalogError::MissingDeclaredAsset(relative.to_string()));
            }
            Err(error) => return Err(error.into()),
        };
        let file_type = metadata.file_type();
        if file_type.is_symlink() || !file_type.is_file() {
            return Err(CatalogError::InvalidDeclaredAsset(relative.to_string()));
        }
    }
    Ok(())
}

fn archive_entries<R: Read + Seek>(
    archive: &mut zip::ZipArchive<R>,
) -> Result<Vec<PackageContentEntry>, CatalogError> {
    let mut entries = Vec::with_capacity(archive.len());
    let mut seen = std::collections::HashSet::new();
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        let path = normalized_archive_path(file.name(), file.is_dir());
        if !seen.insert(archive_path_key(&path)) {
            return Err(CatalogError::DuplicateArchivePath(file.name().to_string()));
        }
        entries.push(PackageContentEntry {
            path,
            uncompressed_size: file.size(),
            is_directory: file.is_dir(),
        });
    }
    Ok(entries)
}

fn normalized_archive_path(name: &str, is_directory: bool) -> PathBuf {
    if is_directory {
        PathBuf::from(name.trim_end_matches('/'))
    } else {
        PathBuf::from(name)
    }
}

fn archive_path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_lowercase()
}

fn directory_entries(root: &Path) -> Result<Vec<PackageContentEntry>, io::Error> {
    let root_metadata = fs::symlink_metadata(root)?;
    if is_filesystem_redirect(&root_metadata) || !root_metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "character package root is not a regular directory: {}",
                root.display()
            ),
        ));
    }
    let mut pending = vec![root.to_path_buf()];
    let mut entries = Vec::new();
    while let Some(directory) = pending.pop() {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            let file_type = metadata.file_type();
            if is_filesystem_redirect(&metadata) {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("character package contains symlink: {}", path.display()),
                ));
            }
            let relative = path
                .strip_prefix(root)
                .expect("walked package paths remain below their root")
                .to_path_buf();
            entries.push(PackageContentEntry {
                path: relative,
                uncompressed_size: if file_type.is_file() {
                    entry.metadata()?.len()
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

fn copy_package_directory(source: &Path, target: &Path) -> Result<(), io::Error> {
    for entry in directory_entries(source)? {
        let source_path = source.join(&entry.path);
        let target_path = target.join(&entry.path);
        if entry.is_directory {
            fs::create_dir_all(target_path)?;
        } else {
            if let Some(parent) = target_path.parent() {
                fs::create_dir_all(parent)?;
            }
            fs::copy(source_path, target_path)?;
        }
    }
    Ok(())
}

fn atomic_replace_directory(staging: &Path, target: &Path) -> Result<(), io::Error> {
    if !target.exists() {
        return fs::rename(staging, target);
    }

    let backup = target.with_file_name(format!(
        ".backup-{}-{}",
        target
            .file_name()
            .and_then(|value| value.to_str())
            .unwrap_or("package"),
        Uuid::new_v4()
    ));
    fs::rename(target, &backup)?;
    match fs::rename(staging, target) {
        Ok(()) => {
            remove_non_redirected_directory(&backup)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, target);
            Err(error)
        }
    }
}

fn remove_non_redirected_directory(path: &Path) -> Result<(), io::Error> {
    if let Some(parent) = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        ensure_catalog_parent_chain(parent, parent)?;
    }
    let metadata = fs::symlink_metadata(path)?;
    if is_filesystem_redirect(&metadata) || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "refusing to remove redirected character directory: {}",
                path.display()
            ),
        ));
    }
    fs::remove_dir_all(path)
}
