// pattern: Imperative Shell

use super::backup::{
    inspect_backup_archive, restore_character_rows, stage_backup_configs,
    stage_character_resources, BackupManifest, CharacterPackageResolver, ConflictStrategy,
    ExportOptions, ImportOptions, LocalCatalogPackageResolver, ResolvedCharacterPackage,
    MAX_BACKUP_RESOURCE_BYTES, MAX_BACKUP_RESOURCE_FILES, MAX_BACKUP_RESOURCE_PACKAGES,
};
use crate::registry::client::sha256_hex;
use crate::registry::manifest::{RegistryEntry, RegistryRecommendations};
use sqlx::sqlite::SqlitePoolOptions;
use std::fs;
use std::io::{Cursor, Write};
use std::path::{Path, PathBuf};

fn create_character_table_sql() -> &'static str {
    "CREATE TABLE characters (id TEXT PRIMARY KEY, name TEXT NOT NULL, persona TEXT NOT NULL DEFAULT '', user_nickname TEXT NOT NULL DEFAULT '', source_format TEXT NOT NULL DEFAULT 'manual', created_at INTEGER NOT NULL DEFAULT 0, updated_at INTEGER NOT NULL DEFAULT 0, template_id TEXT, template_version TEXT, template_snapshot_json TEXT, description TEXT NOT NULL DEFAULT '', avatar_path TEXT, greeting TEXT NOT NULL DEFAULT '', greeting_consumed_at INTEGER, greeting_message_id INTEGER, example_dialogue TEXT NOT NULL DEFAULT '', runtime_profile_json TEXT NOT NULL DEFAULT '{}', user_modified_at INTEGER)"
}

async fn pool_with_characters() -> sqlx::SqlitePool {
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect("sqlite::memory:")
        .await
        .unwrap();
    sqlx::query(create_character_table_sql())
        .execute(&pool)
        .await
        .unwrap();
    pool
}

struct MissingResolver;

impl CharacterPackageResolver for MissingResolver {
    fn resolve_exact(
        &self,
        _template_id: &str,
        _template_version: &str,
    ) -> Result<Option<ResolvedCharacterPackage>, String> {
        Ok(None)
    }
}

#[test]
fn manual_export_defaults_to_data_only_and_old_manifests_remain_compatible() {
    assert!(!ExportOptions::default().include_character_resources);

    let old: BackupManifest = serde_json::from_str(
        r#"{"version":"1","created_at":"2026-07-13T00:00:00Z","app_version":"0.1.0"}"#,
    )
    .unwrap();
    assert!(!old.includes_character_resources);
}

#[test]
fn resource_inclusive_archive_is_detected_separately_from_credentials() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("resources.kokoro");
    let file = fs::File::create(&backup).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    std::io::Write::write_all(
        &mut zip,
        br#"{"version":"2","created_at":"2026-07-13T00:00:00Z","app_version":"0.1.0","includes_character_resources":true}"#,
    )
    .unwrap();
    zip.start_file("character-resources/kokoro/1.0.0/character.json", options)
        .unwrap();
    std::io::Write::write_all(&mut zip, br#"{}"#).unwrap();
    zip.finish().unwrap();

    let inspection = inspect_backup_archive(&backup).unwrap();
    assert!(inspection.has_character_resources);
    assert!(!inspection.includes_provider_credentials);
}

#[tokio::test]
async fn character_table_restore_uses_sqlite_and_falls_back_without_a_package() {
    let source = pool_with_characters().await;
    let target = pool_with_characters().await;
    sqlx::query("INSERT INTO characters (id, name, persona, template_id, template_version, avatar_path, greeting) VALUES ('instance-1', 'Kokoro', 'persona', 'kokoro', '1.0.0', 'avatar.png', 'hello')")
        .execute(&source)
        .await
        .unwrap();

    let restored = restore_character_rows(&target, &source, "overwrite", &MissingResolver)
        .await
        .unwrap();
    assert_eq!(restored, 1);
    let row: (String, Option<String>, String) = sqlx::query_as(
        "SELECT name, avatar_path, greeting FROM characters WHERE id = 'instance-1'",
    )
    .fetch_one(&target)
    .await
    .unwrap();
    assert_eq!(row, ("Kokoro".to_string(), None, "hello".to_string()));
}

#[tokio::test]
async fn managed_instance_avatar_round_trips_as_a_typed_local_reference() {
    let temp = tempfile::tempdir().unwrap();
    let app_data = temp.path();
    let avatar = app_data.join("character-instance-resources/instance-1/avatar.png");
    fs::create_dir_all(avatar.parent().unwrap()).unwrap();
    fs::write(&avatar, [137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
    let source = pool_with_characters().await;
    let target = pool_with_characters().await;
    sqlx::query("INSERT INTO characters (id, name, persona, avatar_path) VALUES ('instance-1', 'PNG', '', 'character-instance-resource://instance-1/avatar.png')")
        .execute(&source)
        .await
        .unwrap();
    let resolver = LocalCatalogPackageResolver::new(app_data.join("characters"));

    restore_character_rows(&target, &source, "overwrite", &resolver)
        .await
        .unwrap();

    let restored: Option<String> =
        sqlx::query_scalar("SELECT avatar_path FROM characters WHERE id = 'instance-1'")
            .fetch_one(&target)
            .await
            .unwrap();
    assert_eq!(
        restored.as_deref(),
        Some("character-instance-resource://instance-1/avatar.png")
    );

    let backup = app_data.join("instance-resource.kokoro");
    write_config_archive(
        &backup,
        &[(
            "character-instance-resources/instance-1/avatar.png",
            &[137, 80, 78, 71, 13, 10, 26, 10],
        )],
    );
    let stage = app_data.join("stage");
    stage_character_resources(&backup, &stage).unwrap();
    assert_eq!(
        fs::read(stage.join(".instances/instance-1/avatar.png")).unwrap(),
        [137, 80, 78, 71, 13, 10, 26, 10]
    );
}

#[test]
fn local_catalog_resolver_requires_an_exact_valid_version() {
    let tmp = tempfile::tempdir().unwrap();
    let package = tmp.path().join("kokoro").join("1.0.0");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("character.json"),
        r#"{"schema_version":1,"engine_version":">=0.1.0","id":"kokoro","version":"1.0.0","name":"Kokoro","description":"desc","author":"team","license":"CC0","avatar":"avatar.png","persona":"persona","greeting":"hello"}"#,
    )
    .unwrap();
    fs::write(package.join("avatar.png"), b"png").unwrap();
    let resolver = LocalCatalogPackageResolver::new(tmp.path().to_path_buf());

    let resolved = resolver.resolve_exact("kokoro", "1.0.0").unwrap().unwrap();
    assert_eq!(resolved.package_dir, package);
    assert_eq!(resolved.avatar_path, Some(PathBuf::from("avatar.png")));
    assert!(resolver.resolve_exact("kokoro", "2.0.0").unwrap().is_none());
}

#[test]
fn backup_resolver_reuses_complete_catalog_validation() {
    let tmp = tempfile::tempdir().unwrap();
    let package = tmp.path().join("kokoro").join("1.0.0");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("character.json"),
        r#"{"schema_version":1,"engine_version":">=99.0.0","id":"kokoro","version":"1.0.0","name":"Kokoro","description":"desc","author":"team","license":"CC0","avatar":"missing.png","persona":"persona","greeting":"hello"}"#,
    )
    .unwrap();
    let resolver = LocalCatalogPackageResolver::new(tmp.path().to_path_buf());

    let error = resolver.resolve_exact("kokoro", "1.0.0").unwrap_err();

    assert!(error.contains("incompatible") || error.contains("declared asset is missing"));
}

#[test]
fn backup_resolver_rejects_any_symlink_encountered_in_package() {
    let tmp = tempfile::tempdir().unwrap();
    let package = tmp.path().join("kokoro").join("1.0.0");
    fs::create_dir_all(&package).unwrap();
    fs::write(
        package.join("character.json"),
        r#"{"schema_version":1,"engine_version":">=0.1.0","id":"kokoro","version":"1.0.0","name":"Kokoro","description":"desc","author":"team","license":"CC0","persona":"persona","greeting":"hello"}"#,
    )
    .unwrap();
    fs::write(package.join("avatar.png"), b"png").unwrap();
    #[cfg(windows)]
    if std::os::windows::fs::symlink_file(package.join("avatar.png"), package.join("linked.png"))
        .is_err()
    {
        return;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(package.join("avatar.png"), package.join("linked.png")).unwrap();
    let resolver = LocalCatalogPackageResolver::new(tmp.path().to_path_buf());

    let error = resolver.resolve_exact("kokoro", "1.0.0").unwrap_err();

    assert!(error.contains("symlink"), "{error}");
}

#[test]
fn official_restore_stages_only_the_verified_exact_package() {
    let bytes = {
        let mut cursor = Cursor::new(Vec::new());
        let mut writer = zip::ZipWriter::new(&mut cursor);
        let options = zip::write::SimpleFileOptions::default();
        writer.start_file("character.json", options).unwrap();
        writer.write_all(br#"{"schema_version":1,"engine_version":">=0.3.1, <0.4.0","id":"kokoro","version":"1.0.0","name":"Kokoro","description":"desc","author":"team","license":"MIT","persona":"persona","greeting":"hello"}"#).unwrap();
        writer.start_file("LICENSE.md", options).unwrap();
        writer.write_all(b"MIT").unwrap();
        writer.finish().unwrap();
        cursor.into_inner()
    };
    let entry = RegistryEntry {
        content_type: "character".to_string(),
        id: "kokoro".to_string(),
        name: "Kokoro".to_string(),
        version: "1.0.0".to_string(),
        author: "team".to_string(),
        description: "desc".to_string(),
        preview: Vec::new(),
        engine_version: ">=0.3.1, <0.4.0".to_string(),
        download_url: "https://raw.githubusercontent.com/chyinan/Kokoro-Engine/main/registry/packages/kokoro-1.0.0.zip".to_string(),
        archive_size: bytes.len() as u64,
        sha256: sha256_hex(&bytes),
        trust: "official".to_string(),
        trust_source: crate::registry::manifest::OFFICIAL_REGISTRY_URL.to_string(),
        registry_identity: Some(crate::registry::manifest::OFFICIAL_REGISTRY_IDENTITY.to_string()),
        permissions: Vec::new(),
        recommendations: RegistryRecommendations { vision: false, memory: false, mcp_servers: Vec::new(), bot_platforms: Vec::new() },
    };

    let temp = tempfile::tempdir().unwrap();
    let resolved = super::backup::stage_verified_official_package(temp.path(), &bytes, &entry)
        .expect("verified official package should stage");
    assert_eq!(resolved.package_dir, temp.path().join("kokoro/1.0.0"));
    assert_eq!(resolved.avatar_path, None);
}

#[tokio::test]
async fn template_restore_prefers_valid_id_matching_managed_avatar() {
    let temp = tempfile::tempdir().unwrap();
    let app_data = temp.path();
    let package = app_data.join("characters/kokoro/1.0.0");
    fs::create_dir_all(&package).unwrap();
    fs::write(package.join("avatar.png"), b"package-avatar").unwrap();
    fs::write(
        package.join("character.json"),
        r#"{"schema_version":1,"engine_version":">=0.1.0","id":"kokoro","version":"1.0.0","name":"Kokoro","description":"desc","author":"team","license":"CC0","avatar":"avatar.png","persona":"persona","greeting":"hello"}"#,
    )
    .unwrap();
    let managed = app_data.join("character-instance-resources/instance-1/avatar.png");
    fs::create_dir_all(managed.parent().unwrap()).unwrap();
    fs::write(&managed, [137, 80, 78, 71, 13, 10, 26, 10]).unwrap();
    let source = pool_with_characters().await;
    let target = pool_with_characters().await;
    sqlx::query("INSERT INTO characters (id, name, persona, template_id, template_version, avatar_path) VALUES ('instance-1', 'Kokoro', '', 'kokoro', '1.0.0', 'character-instance-resource://instance-1/avatar.png')")
        .execute(&source)
        .await
        .unwrap();
    let resolver = LocalCatalogPackageResolver::new(app_data.join("characters"));

    restore_character_rows(&target, &source, "overwrite", &resolver)
        .await
        .unwrap();

    let avatar: Option<String> =
        sqlx::query_scalar("SELECT avatar_path FROM characters WHERE id = 'instance-1'")
            .fetch_one(&target)
            .await
            .unwrap();
    assert_eq!(
        avatar.as_deref(),
        Some("character-instance-resource://instance-1/avatar.png")
    );
}

#[test]
fn invalid_resource_archive_leaves_no_staged_files() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("invalid.kokoro");
    let file = fs::File::create(&backup).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    zip.start_file("manifest.json", options).unwrap();
    std::io::Write::write_all(&mut zip, br#"{"version":"2","created_at":"x","app_version":"x","includes_character_resources":true}"#).unwrap();
    zip.start_file("character-resources/kokoro/1.0.0/../../escape.txt", options)
        .unwrap();
    std::io::Write::write_all(&mut zip, b"bad").unwrap();
    zip.finish().unwrap();
    let stage = tmp.path().join("stage");

    assert!(super::backup::stage_character_resources(&backup, &stage).is_err());
    assert!(!stage.exists());
    assert!(!tmp.path().join("escape.txt").exists());
}

fn write_config_archive(path: &Path, entries: &[(&str, &[u8])]) {
    let file = fs::File::create(path).unwrap();
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::SimpleFileOptions::default();
    for (name, content) in entries {
        zip.start_file(*name, options).unwrap();
        std::io::Write::write_all(&mut zip, content).unwrap();
    }
    zip.finish().unwrap();
}

#[test]
fn config_import_rejects_nested_unknown_and_database_names() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, expected) in [
        ("configs/nested/llm_config.json", "single filename"),
        ("configs/unknown.json", "unknown config"),
        ("configs/kokoro.db", "unknown config"),
    ] {
        let backup = tmp
            .path()
            .join(format!("{}.kokoro", expected.replace(' ', "-")));
        write_config_archive(&backup, &[(name, b"{}")]);

        let error = stage_backup_configs(&backup).unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn config_import_rejects_duplicate_allowed_names() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("duplicate.kokoro");
    write_config_archive(
        &backup,
        &[
            ("configs/llm_config.json", b"{}"),
            ("configs/llm_config.json", b"{}"),
        ],
    );

    let error = stage_backup_configs(&backup).unwrap_err();

    assert!(error.to_string().contains("duplicate config"), "{error}");
}

#[test]
fn config_import_validates_every_payload_before_returning_any() {
    let tmp = tempfile::tempdir().unwrap();
    let backup = tmp.path().join("invalid-json.kokoro");
    write_config_archive(
        &backup,
        &[
            ("configs/llm_config.json", br#"{"provider":"local"}"#),
            ("configs/tts_config.json", b"not-json"),
        ],
    );

    let error = stage_backup_configs(&backup).unwrap_err();

    assert!(error.to_string().contains("invalid JSON config"), "{error}");
}

#[test]
fn resource_archive_enforces_archive_wide_totals_before_extraction() {
    for (packages, files, bytes, expected) in [
        (MAX_BACKUP_RESOURCE_PACKAGES + 1, 1, 1, "package count"),
        (1, MAX_BACKUP_RESOURCE_FILES + 1, 1, "file count"),
        (1, 1, MAX_BACKUP_RESOURCE_BYTES + 1, "uncompressed byte"),
    ] {
        let error =
            super::backup::validate_backup_resource_totals(packages, files, bytes).unwrap_err();
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn staged_instance_avatar_uses_png_signature_and_size_validation() {
    let tmp = tempfile::tempdir().unwrap();
    for (name, bytes, expected) in [
        ("bad-signature", vec![0_u8; 8], "PNG"),
        (
            "too-large",
            {
                let mut bytes = vec![0_u8; 16 * 1024 * 1024 + 1];
                bytes[..8].copy_from_slice(&[137, 80, 78, 71, 13, 10, 26, 10]);
                bytes
            },
            "16 MiB",
        ),
    ] {
        let backup = tmp.path().join(format!("{name}.kokoro"));
        write_config_archive(
            &backup,
            &[("character-instance-resources/instance/avatar.png", &bytes)],
        );

        let error = stage_character_resources(&backup, &tmp.path().join(format!("stage-{name}")))
            .unwrap_err();

        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn import_options_reject_unknown_conflict_strategy() {
    let error = serde_json::from_value::<ImportOptions>(serde_json::json!({
        "import_database": true,
        "import_configs": false,
        "conflict_strategy": "merge",
        "target_character_id": null
    }))
    .unwrap_err();
    assert!(error.to_string().contains("unknown variant"));

    let overwrite = serde_json::from_value::<ImportOptions>(serde_json::json!({
        "import_database": true,
        "import_configs": false,
        "conflict_strategy": "overwrite",
        "target_character_id": null
    }))
    .unwrap();
    assert_eq!(overwrite.conflict_strategy, ConflictStrategy::Overwrite);
}

#[allow(dead_code)]
fn assert_paths_are_local(_: &Path) {}
