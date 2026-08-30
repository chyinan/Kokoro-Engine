// pattern: Functional Core

use crate::characters::manifest::{
    is_supported_package_file, validate_package_path, ManifestError,
};
use std::path::PathBuf;
use thiserror::Error;

pub mod activation;
pub mod catalog;
pub mod instance_resource;
#[cfg(test)]
mod instance_resource_tests;
pub mod manifest;
pub mod merge;

pub const MAX_PACKAGE_FILE_COUNT: usize = 2_048;
pub const MAX_PACKAGE_UNCOMPRESSED_BYTES: u64 = 128 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackageContentEntry {
    pub path: PathBuf,
    pub uncompressed_size: u64,
    pub is_directory: bool,
}

#[derive(Debug, Error)]
pub enum PackageContentError {
    #[error(transparent)]
    InvalidPath(#[from] ManifestError),
    #[error("unsupported package content `{0}`")]
    UnsupportedContent(String),
    #[error("package exceeds file count limit of {MAX_PACKAGE_FILE_COUNT}")]
    FileCountLimit,
    #[error("package exceeds uncompressed size limit of {MAX_PACKAGE_UNCOMPRESSED_BYTES} bytes")]
    UncompressedSizeLimit,
    #[error("package is missing `character.json`")]
    MissingManifest,
    #[error("character package is missing a root license file")]
    MissingRootLicense,
}

pub fn validate_package_content(
    entries: &[PackageContentEntry],
) -> Result<(), PackageContentError> {
    let file_count = entries.iter().filter(|entry| !entry.is_directory).count();
    if file_count > MAX_PACKAGE_FILE_COUNT {
        return Err(PackageContentError::FileCountLimit);
    }

    let mut total_size = 0_u64;
    let mut has_manifest = false;
    let mut has_root_license = false;
    for entry in entries {
        validate_package_path(&entry.path)?;
        if entry.is_directory {
            continue;
        }
        total_size = total_size
            .checked_add(entry.uncompressed_size)
            .ok_or(PackageContentError::UncompressedSizeLimit)?;
        if total_size > MAX_PACKAGE_UNCOMPRESSED_BYTES {
            return Err(PackageContentError::UncompressedSizeLimit);
        }
        if entry.path == std::path::Path::new("character.json") {
            has_manifest = true;
        }
        if is_root_license_file(&entry.path) {
            has_root_license = true;
        }
        if !is_supported_package_file(&entry.path) {
            return Err(PackageContentError::UnsupportedContent(
                entry.path.to_string_lossy().into_owned(),
            ));
        }
    }

    if !has_manifest {
        return Err(PackageContentError::MissingManifest);
    }
    if !has_root_license {
        return Err(PackageContentError::MissingRootLicense);
    }
    Ok(())
}

fn is_root_license_file(path: &std::path::Path) -> bool {
    path.parent()
        .is_some_and(|parent| parent.as_os_str().is_empty())
        && path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                let lower = name.to_ascii_lowercase();
                lower == "license" || lower.starts_with("license.")
            })
}

#[cfg(test)]
mod activation_tests;
#[cfg(test)]
mod catalog_tests;
