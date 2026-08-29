// pattern: Imperative Shell

use crate::characters::catalog::CharacterCatalog;
use crate::registry::client::{
    install_trust, verify_character_archive, InstallTrust, RegistryClientError, MAX_ARCHIVE_BYTES,
};
use semver::Version;
use std::io::{Cursor, Write};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

fn manifest(version: &str, engine: &str) -> String {
    serde_json::json!({
        "schema_version": 1,
        "engine_version": engine,
        "id": "remote-character",
        "version": version,
        "name": "Remote Character",
        "description": "A test character",
        "author": "Kokoro",
        "license": "MIT",
        "persona": "Be helpful.",
        "greeting": "Hello"
    })
    .to_string()
}

fn archive(extra: Option<(&str, &[u8])>, engine: &str) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("character.json", options).unwrap();
        writer
            .write_all(manifest("1.0.0", engine).as_bytes())
            .unwrap();
        writer.start_file("LICENSE.md", options).unwrap();
        writer.write_all(b"MIT").unwrap();
        if let Some((path, data)) = extra {
            writer.start_file(path, options).unwrap();
            writer.write_all(data).unwrap();
        }
        writer.finish().unwrap();
    }
    bytes.into_inner()
}

fn checksum(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(bytes))
}

#[test]
fn rejects_checksum_mismatch_before_install() {
    let bytes = archive(None, ">=0.3.0, <0.4.0");
    let error = verify_character_archive(
        &bytes,
        Some((bytes.len() as u64, "0".repeat(64))),
        &Version::parse("0.3.1").unwrap(),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        RegistryClientError::ChecksumMismatch { .. }
    ));
}

#[test]
fn rejects_incompatible_manifest() {
    let bytes = archive(None, ">=0.4.0");
    let error = verify_character_archive(
        &bytes,
        Some((bytes.len() as u64, checksum(&bytes))),
        &Version::parse("0.3.1").unwrap(),
    )
    .unwrap_err();
    assert!(matches!(error, RegistryClientError::Incompatible(_)));
}

#[test]
fn rejects_corrupt_and_truncated_archives() {
    let bytes = archive(None, ">=0.3.0, <0.4.0");
    for candidate in [b"not-a-zip".to_vec(), bytes[..bytes.len() / 2].to_vec()] {
        let error = verify_character_archive(
            &candidate,
            Some((candidate.len() as u64, checksum(&candidate))),
            &Version::parse("0.3.1").unwrap(),
        )
        .unwrap_err();
        assert!(matches!(error, RegistryClientError::Archive(_)));
    }
}

#[test]
fn rejects_traversal_and_unsupported_script_html_or_executable_files() {
    for path in [
        "../escape.txt",
        "scripts/install.js",
        "view.html",
        "payload.exe",
    ] {
        let bytes = archive(Some((path, b"unsafe")), ">=0.3.0, <0.4.0");
        let error = verify_character_archive(
            &bytes,
            Some((bytes.len() as u64, checksum(&bytes))),
            &Version::parse("0.3.1").unwrap(),
        )
        .unwrap_err();
        assert!(matches!(error, RegistryClientError::UnsafeContent(_)));
    }
}

#[test]
fn rejects_archives_over_download_size_limit() {
    let bytes = archive(None, ">=0.3.0, <0.4.0");
    let error = verify_character_archive(
        &bytes,
        Some((MAX_ARCHIVE_BYTES + 1, checksum(&bytes))),
        &Version::parse("0.3.1").unwrap(),
    )
    .unwrap_err();
    assert!(matches!(error, RegistryClientError::SizeMismatch { .. }));
}

#[test]
fn custom_registry_and_url_installs_cannot_claim_official() {
    assert_eq!(
        install_trust("https://example.test/index.json", true),
        InstallTrust::Community
    );
    assert_eq!(
        install_trust(
            "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json",
            false
        ),
        InstallTrust::Community
    );
    assert_eq!(
        install_trust(
            "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/v1/index.json",
            true
        ),
        InstallTrust::Official
    );
}

#[test]
fn atomic_install_and_remove_preserve_user_data_and_settings() {
    let temp = TempDir::new().unwrap();
    let catalog = CharacterCatalog::new(
        temp.path().join("characters"),
        Version::parse("0.3.1").unwrap(),
    );
    let user_data = temp.path().join("database.sqlite");
    let settings = temp.path().join("context_settings.json");
    std::fs::write(&user_data, b"conversation and memory").unwrap();
    std::fs::write(&settings, b"user override").unwrap();
    catalog
        .install_zip(Cursor::new(archive(None, ">=0.3.0, <0.4.0")))
        .unwrap();
    assert!(catalog.remove_package("remote-character", "1.0.0").is_ok());
    assert_eq!(
        std::fs::read(&user_data).unwrap(),
        b"conversation and memory"
    );
    assert_eq!(std::fs::read(&settings).unwrap(), b"user override");
    assert!(!temp
        .path()
        .join("characters/remote-character/1.0.0")
        .exists());
}

#[test]
fn exact_version_lookup_returns_none_without_breaking_fallback() {
    let temp = TempDir::new().unwrap();
    let catalog = CharacterCatalog::new(
        temp.path().join("characters"),
        Version::parse("0.3.1").unwrap(),
    );
    assert!(catalog.find_exact("remote-character", "9.9.9").is_none());
    assert!(catalog
        .presentation_directory("remote-character", "9.9.9")
        .is_none());
}
