//! Integration test: --target-version 0.5 drops v0.6 artifacts.
//! Also: GUC trigger gating — verifies that setting `secafs.volume_id` on a
//! Postgres session causes the undo triggers to capture inode writes.
//!
//! Skipped if SECAFS_TEST_DATABASE_URL is unset.

use std::env;

/// Verify the GUC trigger gate end-to-end at the SQL level.
///
/// This test does NOT exercise the FUSE driver (no mount), which would require
/// `/dev/fuse`. Instead it directly verifies the contract that
/// `ConnectionPool::set_volume_id_guc` relies on:
///
/// 1. A fresh connection with the GUC unset → trigger short-circuits → no undo row.
/// 2. After `SET secafs.volume_id`, trigger fires → undo row is captured.
///
/// This proves that `set_volume_id_guc` is sufficient for the FUSE driver to
/// enable rollback capture, because the FUSE driver's dedicated pool is
/// indistinguishable (from the trigger's point of view) from the connections
/// used here.
#[tokio::test]
async fn volume_id_guc_triggers_undo_capture() {
    let Some(url) = env::var("SECAFS_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: SECAFS_TEST_DATABASE_URL unset");
        return;
    };

    // ── Reset schema ────────────────────────────────────────────────────────
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    drop(client);

    // ── Bootstrap v0.6 schema (tables + triggers) via SDK ───────────────────
    let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url.clone());
    let sdk = secafs_sdk::SecAFS::open(opts).await.unwrap();
    let conn = sdk.get_connection().await.unwrap();

    // ── Enable rollback for our test volume ─────────────────────────────────
    let volume_id = "guc-trigger-test";
    // Ensure the volume row exists (triggers check fs_volume_state).
    conn.batch_execute(&format!(
        "INSERT INTO fs_volumes (id, root_ino) VALUES ('{volume_id}', 1) ON CONFLICT DO NOTHING"
    )).await.unwrap();
    secafs_sdk::snapshot::enable(&conn, volume_id).await.unwrap();

    // ── 1. Without GUC set — trigger must short-circuit ─────────────────────
    // Use a raw tokio-postgres client to control the GUC precisely.
    let (raw_client, raw_conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = raw_conn.await; });

    // Confirm GUC is empty on a fresh connection.
    let row = raw_client
        .query_one("SELECT current_setting('secafs.volume_id', true)", &[])
        .await
        .unwrap();
    let guc_val: Option<String> = row.get(0);
    assert!(
        guc_val.as_deref().unwrap_or("").is_empty(),
        "GUC should be empty on a fresh connection, got: {guc_val:?}"
    );

    // Insert a dummy inode without the GUC set — trigger should no-op.
    raw_client
        .batch_execute(
            "INSERT INTO fs_inode (ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec)
             VALUES (9001, 0o100644, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)",
        )
        .await
        .unwrap();

    let row = raw_client
        .query_one(
            "SELECT COUNT(*) FROM fs_inode_undo WHERE volume_id = $1",
            &[&volume_id],
        )
        .await
        .unwrap();
    let count_no_guc: i64 = row.get(0);
    assert_eq!(
        count_no_guc, 0,
        "trigger must short-circuit when GUC is unset; got {count_no_guc} undo rows"
    );

    // ── 2. With GUC set — trigger must capture the undo row ─────────────────
    raw_client
        .batch_execute(&format!("SET secafs.volume_id = '{volume_id}'"))
        .await
        .unwrap();

    // Insert a second inode — this time the trigger should fire.
    raw_client
        .batch_execute(
            "INSERT INTO fs_inode (ino, mode, nlink, uid, gid, size, atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec)
             VALUES (9002, 0o100644, 1, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0)",
        )
        .await
        .unwrap();

    let row = raw_client
        .query_one(
            "SELECT COUNT(*) FROM fs_inode_undo WHERE volume_id = $1",
            &[&volume_id],
        )
        .await
        .unwrap();
    let count_with_guc: i64 = row.get(0);
    assert!(
        count_with_guc > 0,
        "trigger must capture undo row when GUC is set; got {count_with_guc} undo rows"
    );

    // ── 3. Verify ConnectionPool::set_volume_id_guc sets GUC on all conns ───
    use std::sync::Arc;
    use secafs_sdk::connection_pool::ConnectionPool;

    let (c1, f1) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    let (c2, f2) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = f1.await; });
    tokio::spawn(async move { let _ = f2.await; });
    let pool = ConnectionPool::new(vec![Arc::new(c1), Arc::new(c2)]);

    pool.set_volume_id_guc(volume_id).await.unwrap();

    // Round-robin: first two calls hit conn[0] and conn[1].
    for _ in 0..2 {
        let conn = pool.get_connection().await.unwrap();
        let mut rows = conn
            .query("SELECT current_setting('secafs.volume_id', true)", ())
            .await
            .unwrap();
        let row = rows.next().await.unwrap().unwrap();
        let val = match row.get_value(0).unwrap() {
            secafs_sdk::db::DbValue::Text(t) => t,
            other => panic!("unexpected DbValue: {other:?}"),
        };
        assert_eq!(
            val, volume_id,
            "set_volume_id_guc must set GUC on every pool connection"
        );
    }
}

#[tokio::test]
async fn downgrade_drops_v0_6_artifacts() {
    let Some(url) = env::var("SECAFS_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: SECAFS_TEST_DATABASE_URL unset");
        return;
    };

    // Reset DB to a fresh v0.6 state.
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    drop(client);

    let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url.clone());
    let _ = secafs_sdk::SecAFS::open(opts).await.unwrap();  // initializes through v0.6

    // Run downgrade via the public CLI handler.
    let mut stdout = Vec::new();
    secafs::cmd::migrate::handle_migrate_command(
        &mut stdout, url.clone(), false, Some("0.5".to_string())
    ).await.unwrap();

    // Verify fs_volume_state is dropped, fs_volumes remains.
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    let row = client.query_one(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public' AND table_name='fs_volume_state'",
        &[],
    ).await.unwrap();
    let count: i64 = row.get(0);
    assert_eq!(count, 0, "fs_volume_state should be dropped");
    let row = client.query_one(
        "SELECT COUNT(*) FROM information_schema.tables WHERE table_schema='public' AND table_name='fs_volumes'",
        &[],
    ).await.unwrap();
    let count: i64 = row.get(0);
    assert_eq!(count, 1, "fs_volumes must remain");
}
