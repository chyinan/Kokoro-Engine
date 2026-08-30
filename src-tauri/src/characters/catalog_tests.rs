// pattern: Imperative Shell

use super::catalog::CharacterCatalog;
use super::{validate_package_content, PackageContentEntry, MAX_PACKAGE_UNCOMPRESSED_BYTES};
use semver::Version;
use std::fs;
use std::io::{Cursor, Write};
use std::path::Path;
use tempfile::TempDir;
use zip::write::SimpleFileOptions;

fn manifest_json(version: &str, greeting: &str) -> String {
    serde_json::json!({
        "schema_version": 1,
        "engine_version": ">=0.3.0, <0.4.0",
        "id": "kokoro",
        "version": version,
        "name": "Kokoro",
        "description": "A warm daily companion",
        "author": "Kokoro Project",
        "license": "CC-BY-4.0",
        "persona": "You are Kokoro.",
        "greeting": greeting
    })
    .to_string()
}

fn manifest_json_with_avatar(version: &str, avatar: &str) -> String {
    let mut manifest: serde_json::Value =
        serde_json::from_str(&manifest_json(version, "Hello")).unwrap();
    manifest["avatar"] = serde_json::json!(avatar);
    manifest.to_string()
}

fn write_package_dir(root: &Path, version: &str, greeting: &str) {
    fs::create_dir_all(root).unwrap();
    fs::write(
        root.join("character.json"),
        manifest_json(version, greeting),
    )
    .unwrap();
    fs::write(root.join("LICENSE.md"), "Test license").unwrap();
}

fn zip_package(version: &str, greeting: &str, extra: Option<(&str, &[u8])>) -> Cursor<Vec<u8>> {
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("character.json", options).unwrap();
        writer
            .write_all(manifest_json(version, greeting).as_bytes())
            .unwrap();
        writer.start_file("LICENSE.md", options).unwrap();
        writer.write_all(b"Test license").unwrap();
        if let Some((path, content)) = extra {
            writer.start_file(path, options).unwrap();
            writer.write_all(content).unwrap();
        }
        writer.finish().unwrap();
    }
    bytes.set_position(0);
    bytes
}

fn test_catalog(temp: &TempDir) -> CharacterCatalog {
    CharacterCatalog::new(
        temp.path().join("app-data-characters"),
        Version::parse("0.3.1").unwrap(),
    )
}

#[test]
fn discovers_valid_versioned_packages_from_app_data() {
    let temp = TempDir::new().unwrap();
    let package = temp
        .path()
        .join("app-data-characters")
        .join("kokoro")
        .join("1.0.0");
    write_package_dir(&package, "1.0.0", "Hello");

    let entries = test_catalog(&temp).discover().unwrap();

    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].manifest.id, "kokoro");
    assert_eq!(entries[0].manifest.version, "1.0.0");
    assert_eq!(entries[0].package_dir, package);
}

#[test]
fn installs_bundled_packages_into_app_data_catalog() {
    let temp = TempDir::new().unwrap();
    let bundled = temp.path().join("bundled");
    write_package_dir(&bundled.join("kokoro"), "1.0.0", "Bundled hello");
    let catalog = test_catalog(&temp);

    let installed = catalog.install_bundled(&bundled).unwrap();

    assert_eq!(installed.len(), 1);
    assert_eq!(installed[0].manifest.greeting, "Bundled hello");
    assert!(temp
        .path()
        .join("app-data-characters/kokoro/1.0.0/character.json")
        .is_file());
}

#[test]
fn rejects_zip_traversal_without_writing_outside_catalog() {
    let temp = TempDir::new().unwrap();
    let catalog = test_catalog(&temp);
    let archive = zip_package("1.0.0", "Hello", Some(("../escape.txt", b"owned")));

    let error = catalog.install_zip(archive).unwrap_err();

    assert!(error.to_string().contains("unsafe package path"));
    assert!(!temp.path().join("escape.txt").exists());
}

#[test]
fn rejects_package_content_over_the_uncompressed_size_limit() {
    let entries = vec![PackageContentEntry {
        path: "avatar.png".into(),
        uncompressed_size: MAX_PACKAGE_UNCOMPRESSED_BYTES + 1,
        is_directory: false,
    }];

    let error = validate_package_content(&entries).unwrap_err();

    assert!(error.to_string().contains("uncompressed size limit"));
}

#[test]
fn rejects_unsupported_archive_content() {
    let temp = TempDir::new().unwrap();
    let archive = zip_package("1.0.0", "Hello", Some(("scripts/install.js", b"alert(1)")));

    let error = test_catalog(&temp).install_zip(archive).unwrap_err();

    assert!(error.to_string().contains("unsupported package content"));
}

#[test]
fn rejects_character_package_without_a_root_license_file() {
    let entries = vec![PackageContentEntry {
        path: "character.json".into(),
        uncompressed_size: 1,
        is_directory: false,
    }];

    let error = validate_package_content(&entries).unwrap_err();

    assert!(error.to_string().contains("root license"));
}

#[test]
fn failed_update_preserves_the_installed_version() {
    let temp = TempDir::new().unwrap();
    let catalog = test_catalog(&temp);
    catalog
        .install_zip(zip_package("1.0.0", "Original greeting", None))
        .unwrap();

    let invalid_update = zip_package(
        "1.0.0",
        "Replacement greeting",
        Some(("component.html", b"not declarative")),
    );
    assert!(catalog.install_zip(invalid_update).is_err());

    let installed = fs::read_to_string(
        temp.path()
            .join("app-data-characters/kokoro/1.0.0/character.json"),
    )
    .unwrap();
    assert!(installed.contains("Original greeting"));
    assert!(!installed.contains("Replacement greeting"));
}

#[test]
fn rejects_manifest_when_a_declared_asset_is_missing() {
    let temp = TempDir::new().unwrap();
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("character.json", options).unwrap();
        writer
            .write_all(manifest_json_with_avatar("1.0.0", "avatar.png").as_bytes())
            .unwrap();
        writer.start_file("LICENSE.md", options).unwrap();
        writer.write_all(b"Test license").unwrap();
        writer.finish().unwrap();
    }
    bytes.set_position(0);

    let error = test_catalog(&temp).install_zip(bytes).unwrap_err();

    assert!(error.to_string().contains("declared asset is missing"));
}

#[test]
fn rejects_manifest_when_a_declared_asset_is_a_directory() {
    let temp = TempDir::new().unwrap();
    let mut bytes = Cursor::new(Vec::new());
    {
        let mut writer = zip::ZipWriter::new(&mut bytes);
        let options = SimpleFileOptions::default();
        writer.start_file("character.json", options).unwrap();
        writer
            .write_all(manifest_json_with_avatar("1.0.0", "avatar.png").as_bytes())
            .unwrap();
        writer.start_file("LICENSE.md", options).unwrap();
        writer.write_all(b"Test license").unwrap();
        writer.add_directory("avatar.png/", options).unwrap();
        writer.finish().unwrap();
    }
    bytes.set_position(0);

    let error = test_catalog(&temp).install_zip(bytes).unwrap_err();

    assert!(error.to_string().contains("regular non-symlink file"));
}
