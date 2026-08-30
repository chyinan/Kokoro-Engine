// pattern: Mixed — pure manifest assertions plus temporary filesystem fixtures

//! TDD coverage for registry-backed MOD installation and lifecycle actions.
//!
//! These tests intentionally exercise the pure validation/staging helpers so
//! filesystem replacement can be verified without constructing a Tauri
//! application handle.

use super::mods::{
    install_mod_archive, install_registry_mod_archive, remove_installed_mod,
    untrusted_mod_url_warning, ModInstallSource,
};
use crate::error::KokoroError;
use crate::mods::manifest::{ModManifest, ModManifestError};
use crate::registry::manifest::{RegistryEntry, RegistryRecommendations};
use semver::Version;
use std::fs;
use std::io::{Cursor, Write};
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

fn manifest_json(id: &str, engine_version: Option<&str>, scripts: &[&str]) -> String {
    serde_json::json!({
        "id": id,
        "name": "Registry Mod",
        "version": "1.0.0",
        "description": "A test MOD",
        "engine_version": engine_version,
        "scripts": scripts,
        "permissions": ["tts"],
        "entry": null,
        "ui_entry": null
    })
    .to_string()
}

fn archive(manifest: &str, extra: Option<(&str, &[u8])>) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("mod.json", options).unwrap();
        writer.write_all(manifest.as_bytes()).unwrap();
        if let Some((path, contents)) = extra {
            writer.start_file(path, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }
    bytes.into_inner()
}

fn archive_with_entries(manifest: &str, entries: &[(&str, &[u8])]) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("mod.json", options).unwrap();
        writer.write_all(manifest.as_bytes()).unwrap();
        for (path, contents) in entries {
            writer.start_file(path, options).unwrap();
            writer.write_all(contents).unwrap();
        }
        writer.finish().unwrap();
    }
    bytes.into_inner()
}

fn archive_with_duplicate_root_manifests(first: &str, second: &str) -> Vec<u8> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("mod.json", options).unwrap();
        writer.write_all(first.as_bytes()).unwrap();
        writer.start_file("mod.json", options).unwrap();
        writer.write_all(second.as_bytes()).unwrap();
        writer.finish().unwrap();
    }
    bytes.into_inner()
}

fn engine() -> Version {
    Version::parse("0.3.1").unwrap()
}

fn create_directory_redirect(link: &std::path::Path, destination: &std::path::Path) -> bool {
    #[cfg(windows)]
    {
        std::os::windows::fs::symlink_dir(destination, link).is_ok()
    }
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(destination, link).is_ok()
    }
}

fn registry_entry(id: &str, version: &str, bytes: &[u8]) -> RegistryEntry {
    use sha2::{Digest, Sha256};
    RegistryEntry {
        content_type: "mod".to_string(),
        id: id.to_string(),
        name: "Registry MOD".to_string(),
        version: version.to_string(),
        author: "Kokoro".to_string(),
        description: "A registry MOD".to_string(),
        preview: Vec::new(),
        engine_version: ">=0.3.0, <0.4.0".to_string(),
        download_url: format!("https://example.test/{id}-{version}.zip"),
        archive_size: bytes.len() as u64,
        sha256: format!("{:x}", Sha256::digest(bytes)),
        trust: "community".to_string(),
        trust_source: "https://example.test/index.json".to_string(),
        registry_identity: None,
        permissions: vec!["tts".to_string()],
        recommendations: RegistryRecommendations {
            vision: false,
            memory: false,
            mcp_servers: Vec::new(),
            bot_platforms: Vec::new(),
        },
    }
}

#[test]
fn engine_compatibility_rejects_incompatible_mod_manifest() {
    let raw = manifest_json("compatibility", Some(">=0.4.0"), &[]);
    let manifest: ModManifest = serde_json::from_str(&raw).unwrap();

    assert!(matches!(
        manifest.validate_for_engine(&engine()),
        Err(ModManifestError::IncompatibleEngine { .. })
    ));
}

#[test]
fn invalid_registry_mod_entry_is_rejected_before_staging() {
    let raw = manifest_json("../escape", Some(">=0.3.0, <0.4.0"), &[]);
    let manifest: ModManifest = serde_json::from_str(&raw).unwrap();
    let temp = TempDir::new().unwrap();

    let result = install_mod_archive(
        &archive(&raw, None),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    );

    assert!(result.is_err());
    assert!(!temp.path().join(manifest.id).exists());
}

#[test]
fn registry_entry_must_be_a_valid_mod_and_match_archive_metadata() {
    let raw = manifest_json("registry-mod", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive(&raw, None);
    let mut entry = registry_entry("registry-mod", "1.0.0", &bytes);
    let temp = TempDir::new().unwrap();
    assert!(install_registry_mod_archive(&bytes, &entry, temp.path(), &engine(), true).is_ok());

    entry.content_type = "character".to_string();
    assert!(install_registry_mod_archive(&bytes, &entry, temp.path(), &engine(), true).is_err());
}

#[test]
fn registry_mod_rejects_entry_engine_range_that_does_not_match_manifest() {
    let raw = manifest_json("registry-engine", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive(&raw, None);
    let mut entry = registry_entry("registry-engine", "1.0.0", &bytes);
    entry.engine_version = ">=0.4.0, <0.5.0".to_string();
    let temp = TempDir::new().unwrap();

    let result = install_registry_mod_archive(&bytes, &entry, temp.path(), &engine(), true);

    assert!(
        result.is_err(),
        "entry and manifest engine ranges must agree"
    );
    assert!(!temp.path().join("registry-engine").exists());
}

#[test]
fn registry_mod_rejects_manifest_permissions_that_do_not_match_index() {
    let raw = manifest_json("permission-drift", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive(&raw, None);
    let mut entry = registry_entry("permission-drift", "1.0.0", &bytes);
    entry.permissions.clear();
    let temp = TempDir::new().unwrap();

    let error = install_registry_mod_archive(&bytes, &entry, temp.path(), &engine(), true)
        .expect_err("registry permissions must be authoritative");

    assert!(error.to_string().contains("permissions"), "{error}");
    assert!(!temp.path().join("permission-drift").exists());
}

#[test]
fn repeated_archive_separators_are_rejected_before_mod_extraction() {
    let raw = manifest_json("repeated-separator", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive_with_entries(&raw, &[("assets//panel.js", b"unsafe alias")]);
    let temp = TempDir::new().unwrap();

    let error = install_mod_archive(
        &bytes,
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Local,
    )
    .expect_err("repeated archive separators must not be aliases");

    assert!(
        error.to_string().contains("separator") || error.to_string().contains("path"),
        "{error}"
    );
    assert!(!temp.path().join("repeated-separator").exists());
}

#[test]
fn registry_mod_requires_explicit_permission_confirmation() {
    let raw = manifest_json("permissioned", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive(&raw, None);
    let entry = registry_entry("permissioned", "1.0.0", &bytes);
    let temp = TempDir::new().unwrap();

    let result = install_registry_mod_archive(&bytes, &entry, temp.path(), &engine(), false);

    assert!(matches!(result, Err(KokoroError::Unauthorized(_))));
    assert!(!temp.path().join("permissioned").exists());
}

#[test]
fn previous_mod_survives_failed_staging_or_permission_review() {
    let temp = TempDir::new().unwrap();
    let previous = temp.path().join("stable");
    fs::create_dir_all(&previous).unwrap();
    fs::write(
        previous.join("mod.json"),
        manifest_json("stable", None, &[]),
    )
    .unwrap();
    fs::write(previous.join("marker.txt"), b"previous release").unwrap();

    let candidate = manifest_json("stable", Some(">=0.3.0, <0.4.0"), &["scripts/main.js"]);
    let result = install_mod_archive(
        &archive(&candidate, Some(("scripts/main.js", b"unsafe"))),
        temp.path(),
        &engine(),
        false,
        ModInstallSource::Url,
    );

    assert!(result.is_err());
    assert_eq!(
        fs::read(previous.join("marker.txt")).unwrap(),
        b"previous release"
    );

    let candidate = manifest_json("stable", Some(">=0.3.0, <0.4.0"), &[]);
    let extraction_failure = install_mod_archive(
        &archive(&candidate, Some(("payload.exe", b"unsafe"))),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    );
    assert!(extraction_failure.is_err());
    assert_eq!(
        fs::read(previous.join("marker.txt")).unwrap(),
        b"previous release"
    );
}

#[test]
fn successful_update_replaces_only_after_staging_and_review() {
    let temp = TempDir::new().unwrap();
    let previous = temp.path().join("stable");
    fs::create_dir_all(&previous).unwrap();
    fs::write(
        previous.join("mod.json"),
        manifest_json("stable", None, &[]),
    )
    .unwrap();
    fs::write(previous.join("marker.txt"), b"previous release").unwrap();

    let candidate = manifest_json("stable", Some(">=0.3.0, <0.4.0"), &[]);
    let result = install_mod_archive(
        &archive(&candidate, None),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    )
    .unwrap();

    assert_eq!(result.manifest.id, "stable");
    assert!(!previous.join("marker.txt").exists());
    assert_eq!(
        fs::read_to_string(previous.join("mod.json")).unwrap(),
        candidate
    );
}

#[test]
fn update_and_remove_commands_keep_user_data_outside_mod_directory() {
    let temp = TempDir::new().unwrap();
    let user_data = temp.path().join("database.sqlite");
    fs::write(&user_data, b"conversation and memory").unwrap();
    let raw = manifest_json("removable", Some(">=0.3.0, <0.4.0"), &[]);
    install_mod_archive(
        &archive(&raw, None),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    )
    .unwrap();

    remove_installed_mod(temp.path(), "removable").unwrap();

    assert!(!temp.path().join("removable").exists());
    assert_eq!(fs::read(&user_data).unwrap(), b"conversation and memory");
}

#[test]
fn url_install_requires_explicit_untrusted_code_warning() {
    let warning = untrusted_mod_url_warning("https://example.test/mod.zip");
    assert!(warning.contains("untrusted"));
    assert!(warning.contains("code"));
    assert!(warning.contains("https://example.test/mod.zip"));
}

#[test]
fn oversized_mod_archive_is_rejected_before_zip_processing() {
    let oversized = vec![0_u8; 64 * 1024 * 1024 + 1];
    let temp = TempDir::new().unwrap();
    let error = install_mod_archive(
        &oversized,
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    )
    .unwrap_err();
    assert!(error.to_string().contains("64MB download limit"));
    assert!(fs::read_dir(temp.path()).unwrap().next().is_none());
}

#[test]
fn mod_archive_rejects_native_executables_and_secret_files_but_allows_js() {
    let temp = TempDir::new().unwrap();
    let manifest = manifest_json("content-policy", Some(">=0.3.0, <0.4.0"), &["main.js"]);
    assert!(install_mod_archive(
        &archive(
            &manifest,
            Some(("main.js", b"export function activate() {}"))
        ),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    )
    .is_ok());

    for path in ["payload.dll", ".env", "secrets.json"] {
        let candidate = archive(
            &manifest_json("content-policy", Some(">=0.3.0, <0.4.0"), &[]),
            Some((path, b"secret")),
        );
        assert!(
            install_mod_archive(
                &candidate,
                temp.path(),
                &engine(),
                true,
                ModInstallSource::Registry,
            )
            .is_err(),
            "unsafe MOD file should be rejected: {path}"
        );
    }
}

#[test]
fn duplicate_root_mod_manifests_are_rejected_before_extraction() {
    let first = manifest_json("duplicate-root", Some(">=0.3.0, <0.4.0"), &[]);
    let second = manifest_json("duplicate-root", Some(">=0.3.0, <0.4.0"), &[]);
    let temp = TempDir::new().unwrap();

    let result = install_mod_archive(
        &archive_with_duplicate_root_manifests(&first, &second),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Registry,
    );

    assert!(result.is_err());
    assert!(!temp.path().join("duplicate-root").exists());
}

#[test]
fn case_folded_duplicate_mod_paths_are_rejected_before_extraction() {
    let raw = manifest_json("case-folded", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive_with_entries(
        &raw,
        &[
            ("assets/panel.js", b"first"),
            ("assets/PANEL.JS", b"second"),
        ],
    );
    let temp = TempDir::new().unwrap();

    let error = install_mod_archive(
        &bytes,
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Local,
    )
    .unwrap_err();

    assert!(error.to_string().contains("duplicate"), "{error}");
    assert!(!temp.path().join("case-folded").exists());
}

#[test]
fn case_folded_duplicate_mod_manifests_are_rejected_before_manifest_lookup() {
    let first = manifest_json("case-manifest", Some(">=0.3.0, <0.4.0"), &[]);
    let second = manifest_json("case-manifest", Some(">=0.3.0, <0.4.0"), &[]);
    let bytes = archive_with_entries(&first, &[("MOD.JSON", second.as_bytes())]);
    let temp = TempDir::new().unwrap();

    let error = install_mod_archive(
        &bytes,
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Local,
    )
    .unwrap_err();

    assert!(error.to_string().contains("duplicate"), "{error}");
    assert!(!temp.path().join("case-manifest").exists());
}

#[test]
fn mod_install_rejects_a_redirected_managed_root_before_writing_outside_it() {
    let temp = TempDir::new().unwrap();
    let managed = temp.path().join("mods");
    let outside = temp.path().join("outside");
    fs::create_dir_all(&outside).unwrap();
    if !create_directory_redirect(&managed, &outside) {
        return;
    }

    let raw = manifest_json("redirected-root", Some(">=0.3.0, <0.4.0"), &[]);
    let error = install_mod_archive(
        &archive(&raw, None),
        &managed,
        &engine(),
        true,
        ModInstallSource::Local,
    )
    .unwrap_err();

    assert!(error.to_string().contains("regular directory"), "{error}");
    assert!(!outside.join(".staging-").exists());
    assert!(fs::read_dir(&outside).unwrap().next().is_none());
}

#[test]
fn mod_remove_rejects_a_redirected_managed_root_without_deleting_outside_data() {
    let temp = TempDir::new().unwrap();
    let managed = temp.path().join("mods");
    let outside = temp.path().join("outside");
    fs::create_dir_all(outside.join("keep-me")).unwrap();
    if !create_directory_redirect(&managed, &outside) {
        return;
    }

    let error = remove_installed_mod(&managed, "keep-me").unwrap_err();

    assert!(error.to_string().contains("regular directory"), "{error}");
    assert!(outside.join("keep-me").is_dir());
}

#[test]
fn mod_install_rejects_case_folded_existing_id_instead_of_replacing_another_mod() {
    let temp = TempDir::new().unwrap();
    let existing = temp.path().join("Foo");
    fs::create_dir_all(&existing).unwrap();
    fs::write(existing.join("sentinel.txt"), b"keep").unwrap();

    let raw = manifest_json("foo", Some(">=0.3.0, <0.4.0"), &[]);
    let error = install_mod_archive(
        &archive(&raw, None),
        temp.path(),
        &engine(),
        true,
        ModInstallSource::Local,
    )
    .unwrap_err();

    assert!(error.to_string().contains("case-insensitive"), "{error}");
    assert_eq!(fs::read(existing.join("sentinel.txt")).unwrap(), b"keep");
}

#[test]
fn mod_install_rejects_a_redirected_parent_of_an_existing_managed_root() {
    let temp = TempDir::new().unwrap();
    let outside = temp.path().join("outside");
    let alias = temp.path().join("managed-alias");
    let managed = alias.join("mods");
    fs::create_dir_all(&outside).unwrap();
    if !create_directory_redirect(&alias, &outside) {
        return;
    }
    fs::create_dir_all(&managed).unwrap();

    let raw = manifest_json("parent-redirect", Some(">=0.3.0, <0.4.0"), &[]);
    let error = install_mod_archive(
        &archive(&raw, None),
        &managed,
        &engine(),
        true,
        ModInstallSource::Local,
    )
    .unwrap_err();

    assert!(error.to_string().contains("regular directory"), "{error}");
    assert!(!outside.join("mods/parent-redirect").exists());
}
