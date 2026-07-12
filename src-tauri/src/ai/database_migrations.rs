// pattern: Imperative Shell

use crate::ai::migration_compatibility::{
    classify_dream_memory_v2_checksum, dream_memory_v2_checksums, ChecksumKind,
};
use anyhow::{bail, Result};
use sqlx::{Row, SqlitePool};

const DREAM_MEMORY_V2_VERSION: i64 = 8;

pub async fn run(pool: &SqlitePool) -> Result<()> {
    repair_compatible_migration_checksums(pool).await?;
    sqlx::migrate!("./migrations").run(pool).await?;
    Ok(())
}

async fn repair_compatible_migration_checksums(pool: &SqlitePool) -> Result<()> {
    let migrations_table_exists: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = '_sqlx_migrations')",
    )
    .fetch_one(pool)
    .await?;
    if !migrations_table_exists {
        return Ok(());
    }

    let Some(row) =
        sqlx::query("SELECT checksum FROM _sqlx_migrations WHERE version = ? AND success = TRUE")
            .bind(DREAM_MEMORY_V2_VERSION)
            .fetch_optional(pool)
            .await?
    else {
        return Ok(());
    };
    let stored_checksum: Vec<u8> = row.try_get("checksum")?;
    let checksums = dream_memory_v2_checksums();

    match classify_dream_memory_v2_checksum(&stored_checksum, &checksums) {
        ChecksumKind::Current | ChecksumKind::Unknown => return Ok(()),
        ChecksumKind::CompatibleLineEndingVariant => {}
    }

    if !dream_memory_v2_schema_is_complete(pool).await? {
        bail!(
            "refusing to repair migration 8 checksum because the dream memory v2 schema is incomplete"
        );
    }

    let result = sqlx::query(
        "UPDATE _sqlx_migrations SET checksum = ? \
         WHERE version = ? AND success = TRUE AND checksum = ?",
    )
    .bind(checksums.current.as_slice())
    .bind(DREAM_MEMORY_V2_VERSION)
    .bind(checksums.crlf.as_slice())
    .execute(pool)
    .await?;

    if result.rows_affected() != 1 {
        bail!("migration 8 checksum changed while compatibility repair was running");
    }

    tracing::warn!(
        target: "database",
        "normalized migration 8 checksum after detecting an LF/CRLF-only difference"
    );
    Ok(())
}

async fn dream_memory_v2_schema_is_complete(pool: &SqlitePool) -> Result<bool> {
    let memory_column_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM pragma_table_info('memories') \
         WHERE name IN ('memory_type', 'entity_key', 'status', 'canonical_hash', 'last_dreamed_at')",
    )
    .fetch_one(pool)
    .await?;
    let dream_table_count: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM sqlite_master WHERE type = 'table' \
         AND name IN ('memory_candidates', 'memory_evidence', 'memory_dream_jobs', \
                      'memory_dream_proposals', 'memory_operations')",
    )
    .fetch_one(pool)
    .await?;

    Ok(memory_column_count == 5 && dream_table_count == 5)
}

#[cfg(test)]
mod tests {
    use super::repair_compatible_migration_checksums;
    use crate::ai::migration_compatibility::dream_memory_v2_checksums;
    use sqlx::{Row, SqlitePool};

    #[tokio::test]
    async fn repairs_crlf_checksum_when_dream_memory_schema_is_complete() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let checksums = dream_memory_v2_checksums();
        sqlx::query("UPDATE _sqlx_migrations SET checksum = ? WHERE version = 8")
            .bind(checksums.crlf.as_slice())
            .execute(&pool)
            .await
            .unwrap();

        repair_compatible_migration_checksums(&pool).await.unwrap();

        let checksum: Vec<u8> =
            sqlx::query("SELECT checksum FROM _sqlx_migrations WHERE version = 8")
                .fetch_one(&pool)
                .await
                .unwrap()
                .get("checksum");
        assert_eq!(checksum, checksums.current);
    }

    #[tokio::test]
    async fn rejects_checksum_repair_when_dream_memory_schema_is_incomplete() {
        let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
        sqlx::query(
            "CREATE TABLE _sqlx_migrations (\
                version BIGINT PRIMARY KEY, description TEXT NOT NULL, installed_on TIMESTAMP NOT NULL, \
                success BOOLEAN NOT NULL, checksum BLOB NOT NULL, execution_time BIGINT NOT NULL\
            )",
        )
        .execute(&pool)
        .await
        .unwrap();
        let checksums = dream_memory_v2_checksums();
        sqlx::query(
            "INSERT INTO _sqlx_migrations \
             (version, description, installed_on, success, checksum, execution_time) \
             VALUES (8, 'dream memory v2', CURRENT_TIMESTAMP, TRUE, ?, 0)",
        )
        .bind(checksums.crlf.as_slice())
        .execute(&pool)
        .await
        .unwrap();

        let error = repair_compatible_migration_checksums(&pool)
            .await
            .unwrap_err();

        assert!(error.to_string().contains("schema is incomplete"));
    }
}
