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
        if let Err(error) = fs::remove_dir_all(&staging) {
            let _ = fs::rename(&staging, &self.target);
            return Err(error.into());
        }
        Ok(())
    }

    fn rollback_inner(&mut self) -> Result<(), CatalogError> {
        let Some(staging) = self.staging.take() else {
            return Ok(());
        };
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
        if !is_safe_catalog_segment(id) || !is_safe_catalog_segment(version) {
            return None;
        }
        let target = self.root.join(id).join(version);
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
        if !is_safe_catalog_segment(id) || !is_safe_catalog_segment(version) {
            return Err(CatalogError::Layout {
                id: id.to_string(),
                version: version.to_string(),
            });
        }
        let target = self.root.join(id).join(version);
        let metadata = match fs::symlink_metadata(&target) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        if metadata.file_type().is_symlink() || !metadata.is_dir() {
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
        fs::create_dir_all(&self.root)?;
        let mut packages = Vec::new();
        for id_entry in fs::read_dir(&self.root)? {
            let id_entry = id_entry?;
            if !id_entry.file_type()?.is_dir()
                || id_entry.file_name().to_string_lossy().starts_with('.')
            {
                continue;
            }
            for version_entry in fs::read_dir(id_entry.path())? {
                let version_entry = version_entry?;
                if !version_entry.file_type()?.is_dir()
                    || version_entry.file_name().to_string_lossy().starts_with('.')
                {
                    continue;
                }
                match self.validate_installed_directory(&version_entry.path()) {
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
        fs::create_dir_all(&self.root)?;
        if !bundled_root.is_dir() {
            return Ok(Vec::new());
        }

        let mut installed = Vec::new();
        for package in fs::read_dir(bundled_root)? {
            let package = package?;
            if package.file_type()?.is_dir() && package.path().join("character.json").is_file() {
                installed.push(self.install_directory(&package.path())?);
            }
        }
        installed.sort_by(|left, right| left.manifest.id.cmp(&right.manifest.id));
        Ok(installed)
    }

    pub fn install_zip<R: Read + Seek>(&self, reader: R) -> Result<CatalogEntry, CatalogError> {
        fs::create_dir_all(&self.root)?;
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
            let _ = fs::remove_dir_all(&staging);
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
            let _ = fs::remove_dir_all(&staging);
            return Err(error.into());
        }
        self.commit_validated_staging(staging, manifest)
    }

    fn commit_staging(&self, staging: PathBuf) -> Result<CatalogEntry, CatalogError> {
        let manifest = match self.validate_source_directory(&staging) {
            Ok(manifest) => manifest,
            Err(error) => {
                let _ = fs::remove_dir_all(&staging);
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
        if let Some(parent) = target.parent() {
            fs::create_dir_all(parent)?;
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

fn is_safe_catalog_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value != "."
        && value != ".."
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-' || byte == b'.'
        })
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
    for index in 0..archive.len() {
        let file = archive.by_index(index)?;
        entries.push(PackageContentEntry {
            path: normalized_archive_path(file.name(), file.is_dir()),
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

fn directory_entries(root: &Path) -> Result<Vec<PackageContentEntry>, io::Error> {
    let root_type = fs::symlink_metadata(root)?.file_type();
    if root_type.is_symlink() || !root_type.is_dir() {
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
            let file_type = entry.file_type()?;
            let path = entry.path();
            if file_type.is_symlink() {
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
            fs::remove_dir_all(backup)?;
            Ok(())
        }
        Err(error) => {
            let _ = fs::rename(&backup, target);
            Err(error)
        }
    }
}
