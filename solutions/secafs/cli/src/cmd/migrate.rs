//! Database schema migration command.
//!
//! Migrates a SecAFS PostgreSQL database to the current schema version.

use secafs_sdk::{SecAFSOptions, SchemaVersion, SECAFS_SCHEMA_VERSION};
use anyhow::{Context, Result as AnyhowResult};
use std::io::Write;

/// Handle the migrate command.
pub async fn handle_migrate_command(
    stdout: &mut impl Write,
    id_or_path: String,
    dry_run: bool,
    target_version: Option<String>,
) -> AnyhowResult<()> {
    let options = SecAFSOptions::resolve(&id_or_path)?;

    writeln!(stdout, "Database: {}", id_or_path)?;

    // Open a connection via the SDK to detect the schema version.
    // SecAFS::open() would fail on version mismatch, so we use a
    // low-level pool connection to introspect first.
    let pg_url = options
        .postgres_url
        .as_ref()
        .context("migrate requires a PostgreSQL URL")?;

    let (client, connection) =
        tokio_postgres::connect(pg_url, tokio_postgres::NoTls).await?;
    tokio::spawn(async move {
        if let Err(e) = connection.await {
            tracing::error!("postgres connection error: {e}");
        }
    });

    let pool = secafs_sdk::connection_pool::ConnectionPool::new(
        vec![std::sync::Arc::new(client)],
    );
    let conn = pool.get_connection().await?;

    let current_version = secafs_sdk::schema::detect_schema_version(&conn)
        .await?
        .unwrap_or(SchemaVersion::V0_0);
    writeln!(stdout, "Current schema version: {}", current_version)?;

    // Handle downgrade path.
    if let Some(ref tv) = target_version {
        match tv.as_str() {
            "0.5" => {
                if current_version != SchemaVersion::V0_6 {
                    anyhow::bail!(
                        "Downgrade to v0.5 requires current version to be v0.6, but current is {}",
                        current_version
                    );
                }
                downgrade_v0_6_to_v0_5(&conn, stdout).await?;
                writeln!(stdout, "\nDowngrade completed successfully.")?;
                return Ok(());
            }
            other => {
                anyhow::bail!("Unsupported --target-version: {}. Only '0.5' is supported for downgrade.", other);
            }
        }
    }

    writeln!(stdout, "Target schema version: {}", SECAFS_SCHEMA_VERSION)?;

    if current_version.is_current() {
        writeln!(stdout, "Database is already at the latest schema version.")?;
        return Ok(());
    }

    if dry_run {
        writeln!(
            stdout,
            "\n[DRY RUN] The following migrations would be applied:"
        )?;
        print_pending_migrations(stdout, current_version)?;
        writeln!(stdout, "\nRun without --dry-run to apply migrations.")?;
    } else {
        writeln!(stdout, "\nApplying migrations...")?;
        apply_migrations(&conn, current_version, stdout).await?;

        conn.execute(
            "INSERT INTO fs_config (key, value) VALUES ('schema_version', $1) \
             ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            (SECAFS_SCHEMA_VERSION,),
        )
        .await
        .context("Failed to store schema version")?;

        writeln!(stdout, "\nMigration completed successfully.")?;
    }

    Ok(())
}

async fn downgrade_v0_6_to_v0_5(
    conn: &secafs_sdk::db::DbConn,
    stdout: &mut impl Write,
) -> AnyhowResult<()> {
    writeln!(stdout, "Downgrading v0.6 -> v0.5 (drops snapshot tables + triggers)")?;
    let drops = [
        "DROP TRIGGER IF EXISTS tr_fs_inode_undo ON fs_inode",
        "DROP TRIGGER IF EXISTS tr_fs_dentry_undo ON fs_dentry",
        "DROP TRIGGER IF EXISTS tr_fs_data_undo ON fs_data",
        "DROP TRIGGER IF EXISTS tr_fs_symlink_undo ON fs_symlink",
        "DROP TRIGGER IF EXISTS tr_kv_store_undo ON kv_store",
        "DROP FUNCTION IF EXISTS fs_inode_capture_undo()",
        "DROP FUNCTION IF EXISTS fs_dentry_capture_undo()",
        "DROP FUNCTION IF EXISTS fs_data_capture_undo()",
        "DROP FUNCTION IF EXISTS fs_symlink_capture_undo()",
        "DROP FUNCTION IF EXISTS kv_store_capture_undo()",
        "DROP TABLE IF EXISTS fs_inode_undo, fs_dentry_undo, fs_data_undo, fs_symlink_undo, kv_store_undo",
        "DROP TABLE IF EXISTS fs_snapshots, fs_volume_state",
    ];
    for stmt in drops {
        conn.execute(stmt, ()).await.with_context(|| stmt.to_string())?;
    }
    conn.execute(
        "INSERT INTO fs_config (key, value) VALUES ('schema_version', '0.5') \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
        (),
    ).await?;
    Ok(())
}

fn print_pending_migrations(
    stdout: &mut impl Write,
    from_version: SchemaVersion,
) -> AnyhowResult<()> {
    match from_version {
        SchemaVersion::V0_0 => {
            writeln!(stdout, "  - v0.0 -> v0.2: Add nlink column to fs_inode")?;
            writeln!(stdout, "  - v0.2 -> v0.4: Add atime_nsec, mtime_nsec, ctime_nsec, rdev columns to fs_inode")?;
            writeln!(stdout, "  - v0.4 -> v0.5: Add fs_volumes table")?;
            writeln!(stdout, "  - v0.5 -> v0.6: add snapshot tables and triggers")?;
        }
        SchemaVersion::V0_2 => {
            writeln!(stdout, "  - v0.2 -> v0.4: Add atime_nsec, mtime_nsec, ctime_nsec, rdev columns to fs_inode")?;
            writeln!(stdout, "  - v0.4 -> v0.5: Add fs_volumes table")?;
            writeln!(stdout, "  - v0.5 -> v0.6: add snapshot tables and triggers")?;
        }
        SchemaVersion::V0_4 => {
            writeln!(stdout, "  - v0.4 -> v0.5: Add fs_volumes table")?;
            writeln!(stdout, "  - v0.5 -> v0.6: add snapshot tables and triggers")?;
        }
        SchemaVersion::V0_5 => {
            writeln!(stdout, "  - v0.5 -> v0.6: add snapshot tables and triggers")?;
        }
        SchemaVersion::V0_6 => {}
    }
    Ok(())
}

async fn apply_migrations(
    conn: &secafs_sdk::db::DbConn,
    from_version: SchemaVersion,
    stdout: &mut impl Write,
) -> AnyhowResult<()> {
    match from_version {
        SchemaVersion::V0_0 => {
            migrate_v0_0_to_v0_2(conn, stdout).await?;
            migrate_v0_2_to_v0_4(conn, stdout).await?;
            migrate_v0_4_to_v0_5(conn, stdout).await?;
            migrate_v0_5_to_v0_6(conn, stdout).await?;
        }
        SchemaVersion::V0_2 => {
            migrate_v0_2_to_v0_4(conn, stdout).await?;
            migrate_v0_4_to_v0_5(conn, stdout).await?;
            migrate_v0_5_to_v0_6(conn, stdout).await?;
        }
        SchemaVersion::V0_4 => {
            migrate_v0_4_to_v0_5(conn, stdout).await?;
            migrate_v0_5_to_v0_6(conn, stdout).await?;
        }
        SchemaVersion::V0_5 => {
            migrate_v0_5_to_v0_6(conn, stdout).await?;
        }
        SchemaVersion::V0_6 => {}
    }
    Ok(())
}

async fn migrate_v0_0_to_v0_2(
    conn: &secafs_sdk::db::DbConn,
    stdout: &mut impl Write,
) -> AnyhowResult<()> {
    writeln!(stdout, "  Migrating v0.0 -> v0.2...")?;

    let result = conn
        .execute(
            "ALTER TABLE fs_inode ADD COLUMN nlink BIGINT NOT NULL DEFAULT 0",
            (),
        )
        .await;

    match result {
        Ok(_) => writeln!(stdout, "    Added nlink column to fs_inode")?,
        Err(e) => {
            let err_msg = e.to_string();
            if err_msg.contains("already exists") || err_msg.contains("duplicate column") {
                writeln!(stdout, "    nlink column already exists (skipping)")?;
            } else {
                return Err(e).context("Failed to add nlink column");
            }
        }
    }

    writeln!(stdout, "  v0.0 -> v0.2 migration complete.")?;
    Ok(())
}

async fn migrate_v0_2_to_v0_4(
    conn: &secafs_sdk::db::DbConn,
    stdout: &mut impl Write,
) -> AnyhowResult<()> {
    writeln!(stdout, "  Migrating v0.2 -> v0.4...")?;

    for (col, sql) in [
        ("atime_nsec", "ALTER TABLE fs_inode ADD COLUMN atime_nsec BIGINT NOT NULL DEFAULT 0"),
        ("mtime_nsec", "ALTER TABLE fs_inode ADD COLUMN mtime_nsec BIGINT NOT NULL DEFAULT 0"),
        ("ctime_nsec", "ALTER TABLE fs_inode ADD COLUMN ctime_nsec BIGINT NOT NULL DEFAULT 0"),
        ("rdev", "ALTER TABLE fs_inode ADD COLUMN rdev BIGINT NOT NULL DEFAULT 0"),
    ] {
        let result = conn.execute(sql, ()).await;
        match result {
            Ok(_) => writeln!(stdout, "    Added {} column to fs_inode", col)?,
            Err(e) => {
                let err_msg = e.to_string();
                if err_msg.contains("already exists") || err_msg.contains("duplicate column") {
                    writeln!(stdout, "    {} column already exists (skipping)", col)?;
                } else {
                    return Err(e).context(format!("Failed to add {} column", col));
                }
            }
        }
    }

    writeln!(stdout, "  v0.2 -> v0.4 migration complete.")?;
    Ok(())
}

async fn migrate_v0_4_to_v0_5(
    conn: &secafs_sdk::db::DbConn,
    stdout: &mut impl Write,
) -> AnyhowResult<()> {
    writeln!(stdout, "  Migrating v0.4 -> v0.5...")?;

    conn.execute(
        "CREATE TABLE IF NOT EXISTS fs_volumes (\n\
            id TEXT PRIMARY KEY,\n\
            root_ino BIGINT NOT NULL REFERENCES fs_inode(ino) ON DELETE CASCADE,\n\
            created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()\n\
         )",
        (),
    )
    .await
    .context("Failed to create fs_volumes table")?;
    writeln!(stdout, "    Created/verified fs_volumes table")?;

    conn.execute(
        "CREATE INDEX IF NOT EXISTS fs_volumes_root_ino_idx ON fs_volumes(root_ino)",
        (),
    )
    .await
    .ok();

    writeln!(stdout, "  v0.4 -> v0.5 migration complete.")?;
    Ok(())
}

async fn migrate_v0_5_to_v0_6(
    conn: &secafs_sdk::db::DbConn,
    stdout: &mut impl Write,
) -> AnyhowResult<()> {
    writeln!(stdout, "Applying v0.5 -> v0.6 migration...")?;
    for stmt in secafs_sdk::snapshot::schema::ddl_statements() {
        conn.batch_execute(stmt).await.with_context(|| format!("snapshot schema: {stmt}"))?;
    }
    for stmt in secafs_sdk::snapshot::triggers::ddl_statements() {
        conn.batch_execute(stmt).await.with_context(|| format!("snapshot trigger: {stmt}"))?;
    }
    writeln!(stdout, "  v0.5 -> v0.6 migration complete")?;
    Ok(())
}
