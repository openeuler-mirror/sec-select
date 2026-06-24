//! Smoke test for v0.5 → v0.6 migration.
//!
//! Requires a running test PG (the existing `secafs-test-pg` container).
//! Skipped if `SECAFS_TEST_DATABASE_URL` is unset.

use std::env;

#[tokio::test]
async fn applies_v0_6_tables_and_triggers() {
    let Some(url) = env::var("SECAFS_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: SECAFS_TEST_DATABASE_URL unset");
        return;
    };

    // Connect, drop public schema, recreate, apply v0.5-equivalent state, then
    // apply v0.6 statements and assert the new tables/triggers exist.
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });

    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();

    // Apply v0.5 baseline: open SecAFS once which initializes through the latest
    // schema (V0_6 once Step 4 is implemented).
    drop(client);
    let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url.clone());
    let _fs = secafs_sdk::SecAFS::open(opts).await.unwrap();

    // Apply v0.6 DDL idempotently (should be a no-op since SecAFS::open already did it).
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    for stmt in secafs_sdk::snapshot::schema::ddl_statements() {
        client.batch_execute(stmt).await.expect(stmt);
    }
    for stmt in secafs_sdk::snapshot::triggers::ddl_statements() {
        client.batch_execute(stmt).await.expect(stmt);
    }

    // Verify tables exist.
    for tbl in [
        "fs_volume_state",
        "fs_snapshots",
        "fs_inode_undo",
        "fs_dentry_undo",
        "fs_data_undo",
        "fs_symlink_undo",
        "kv_store_undo",
    ] {
        let row = client
            .query_one(
                "SELECT 1 FROM information_schema.tables WHERE table_schema='public' AND table_name=$1",
                &[&tbl],
            )
            .await
            .unwrap_or_else(|_| panic!("table {tbl} missing"));
        let _: i32 = row.get(0);
    }

    // Verify the 5 triggers exist.
    for trig in [
        "tr_fs_inode_undo",
        "tr_fs_dentry_undo",
        "tr_fs_data_undo",
        "tr_fs_symlink_undo",
        "tr_kv_store_undo",
    ] {
        let row = client
            .query_one(
                "SELECT 1 FROM pg_trigger WHERE tgname=$1 AND NOT tgisinternal",
                &[&trig],
            )
            .await
            .unwrap_or_else(|_| panic!("trigger {trig} missing"));
        let _: i32 = row.get(0);
    }
}

#[tokio::test]
async fn enable_disable_purges_state() {
    let Some(url) = std::env::var("SECAFS_TEST_DATABASE_URL").ok() else {
        eprintln!("skipped: SECAFS_TEST_DATABASE_URL unset");
        return;
    };

    // Reset to a clean state so SecAFS::open can initialize the full v0.6 schema.
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    drop(client);

    let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url);
    let fs = secafs_sdk::SecAFS::open(opts).await.unwrap();
    let conn = fs.get_connection().await.unwrap();

    let vol_id = "test-vol-state";
    secafs_sdk::filesystem::volume::ensure(&conn, vol_id).await.unwrap();

    let state = secafs_sdk::snapshot::enable(&conn, vol_id).await.unwrap();
    assert!(state.rollback_enabled);
    assert_eq!(state.current_snap_id, 1);

    let result = secafs_sdk::snapshot::disable(&conn, vol_id).await.unwrap();
    assert_eq!(result.purged_snapshots, 0);

    let after = secafs_sdk::snapshot::get_state(&conn, vol_id).await.unwrap().unwrap();
    assert!(!after.rollback_enabled);
}

#[tokio::test]
async fn commit_increments_snap_and_lists() {
    let Some(url) = std::env::var("SECAFS_TEST_DATABASE_URL").ok() else { return; };
    // Reset DB so this test does not collide with other tests' state.
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    drop(client);

    let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url);
    let fs = secafs_sdk::SecAFS::open(opts).await.unwrap();
    let conn = fs.get_connection().await.unwrap();

    let vol_id = "test-vol-snap";
    secafs_sdk::filesystem::volume::ensure(&conn, vol_id).await.unwrap();
    secafs_sdk::snapshot::enable(&conn, vol_id).await.unwrap();

    let s1 = secafs_sdk::snapshot::commit(&conn, vol_id, Some("msg-1")).await.unwrap();
    let s2 = secafs_sdk::snapshot::commit(&conn, vol_id, Some("msg-2")).await.unwrap();
    assert_eq!(s1.snap_id, 1);
    assert_eq!(s2.snap_id, 2);

    // Idempotent on duplicate label.
    let s1_again = secafs_sdk::snapshot::commit(&conn, vol_id, Some("msg-1")).await.unwrap();
    assert_eq!(s1_again.snap_id, 1);

    let list = secafs_sdk::snapshot::list(&conn, vol_id).await.unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(list[0].snap_id, 1);
    assert_eq!(list[0].label.as_deref(), Some("msg-1"));
}

#[tokio::test]
async fn restore_recovers_baseline_after_simple_changes() {
    let Some(url) = std::env::var("SECAFS_TEST_DATABASE_URL").ok() else { return; };
    // Reset DB for isolation.
    let (client, conn) = tokio_postgres::connect(&url, tokio_postgres::NoTls).await.unwrap();
    tokio::spawn(async move { let _ = conn.await; });
    client.batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;").await.unwrap();
    drop(client);

    let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url);
    let fs = secafs_sdk::SecAFS::open(opts).await.unwrap();
    let conn = fs.get_connection().await.unwrap();

    let vol = "test-restore";
    secafs_sdk::filesystem::volume::ensure(&conn, vol).await.unwrap();
    secafs_sdk::snapshot::enable(&conn, vol).await.unwrap();

    // Set GUC manually so trigger captures undo for our test writes.
    conn.execute(&format!("SET secafs.volume_id = '{vol}'"), ()).await.unwrap();

    // Create a file inode + dentry.
    conn.execute(
        "INSERT INTO fs_inode (mode, atime, mtime, ctime) VALUES (33188, 0, 0, 0)",
        (),
    ).await.unwrap();
    let mut rows = conn.query("SELECT MAX(ino) FROM fs_inode", ()).await.unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let ino = match row.get_value(0).unwrap() {
        secafs_sdk::db::DbValue::Integer(i) => i,
        _ => panic!("expected ino integer"),
    };
    conn.execute(
        "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES ('hello.txt', 1, $1)",
        (ino,),
    ).await.unwrap();

    let s1 = secafs_sdk::snapshot::commit(&conn, vol, Some("baseline")).await.unwrap();

    // Mutate.
    conn.execute("UPDATE fs_inode SET size = 42 WHERE ino = $1", (ino,)).await.unwrap();
    conn.execute("DELETE FROM fs_dentry WHERE ino = $1", (ino,)).await.unwrap();
    secafs_sdk::snapshot::commit(&conn, vol, Some("after-changes")).await.unwrap();

    // Restore.
    let outcome = secafs_sdk::snapshot::restore_to(&conn, vol, s1.snap_id).await.unwrap();
    assert_eq!(outcome.pruned_snapshots, 1);
    assert!(outcome.pruned_undo_rows >= 2);

    // Verify state matches baseline.
    let mut rows = conn.query("SELECT size FROM fs_inode WHERE ino = $1", (ino,)).await.unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let size = match row.get_value(0).unwrap() {
        secafs_sdk::db::DbValue::Integer(i) => i,
        _ => panic!(),
    };
    assert_eq!(size, 0, "size should be back to 0");

    let mut rows = conn.query("SELECT COUNT(*) FROM fs_dentry WHERE ino = $1", (ino,)).await.unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let count = match row.get_value(0).unwrap() {
        secafs_sdk::db::DbValue::Integer(i) => i,
        _ => panic!(),
    };
    assert_eq!(count, 1, "dentry should be restored");
}
