use secafs_sdk::connection_pool::ConnectionPool;
use secafs_sdk::db::DbConn;
use secafs_sdk::filesystem::secafs::SecAFS;
use secafs_sdk::filesystem::volume;
use std::sync::Arc;
use uuid::Uuid;

/// Connect to Postgres, create a fresh isolated schema, run `initialize_schema`,
/// and return a connection that points to that schema.
async fn fresh_db() -> DbConn {
    let url = std::env::var("SECAFS_TEST_POSTGRES_URL")
        .expect("SECAFS_TEST_POSTGRES_URL required");

    // Use a UUID-based schema name to avoid collisions under concurrent test execution.
    let schema = format!("vol_test_{}", Uuid::new_v4().simple());

    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("failed to connect to postgres");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres connection error: {}", e);
        }
    });

    // Scope to this schema so the tables don't collide with other tests.
    client
        .execute(
            &format!("CREATE SCHEMA IF NOT EXISTS \"{}\"", schema),
            &[],
        )
        .await
        .unwrap();
    client
        .execute(
            &format!("SET search_path TO \"{}\"", schema),
            &[],
        )
        .await
        .unwrap();

    let client = Arc::new(client);
    let pool = ConnectionPool::new(vec![Arc::clone(&client)]);
    let conn = pool.get_connection().await.unwrap();

    // search_path is per-session; the pool reuses the same client so the
    // setting persists for all queries via this connection.
    SecAFS::initialize_schema(&conn).await.unwrap();

    conn
}

#[tokio::test]
async fn ensure_is_idempotent() {
    let conn = fresh_db().await;
    let root1 = volume::ensure(&conn, "conv-A").await.unwrap();
    let root2 = volume::ensure(&conn, "conv-A").await.unwrap();
    assert_eq!(root1, root2, "repeated ensure returns stable root_ino");
    assert_ne!(root1, 1, "volume root_ino must not collide with global root (ino=1)");
}

#[tokio::test]
async fn different_ids_get_different_roots() {
    let conn = fresh_db().await;
    let a = volume::ensure(&conn, "conv-A").await.unwrap();
    let b = volume::ensure(&conn, "conv-B").await.unwrap();
    assert_ne!(a, b);
}

#[tokio::test]
async fn destroy_unknown_returns_false() {
    let conn = fresh_db().await;
    let destroyed = volume::destroy(&conn, "ghost").await.unwrap();
    assert!(!destroyed);
}

#[tokio::test]
async fn destroy_cleans_fs_volumes_row() {
    let conn = fresh_db().await;
    volume::ensure(&conn, "conv-A").await.unwrap();
    let destroyed = volume::destroy(&conn, "conv-A").await.unwrap();
    assert!(destroyed);
    // re-ensure should create a fresh inode
    let fresh = volume::ensure(&conn, "conv-A").await.unwrap();
    assert!(fresh >= 1);
}

#[tokio::test]
async fn destroy_cleans_fs_data_and_fs_symlink() {
    let conn = fresh_db().await;
    let root_ino = volume::ensure(&conn, "conv-X").await.unwrap();

    // Insert a child inode under the volume root.
    let now: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let mut rows = conn
        .query(
            "INSERT INTO fs_inode \
             (mode, nlink, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec) \
             VALUES (33188, 1, 0, 0, 0, ?, ?, ?, 0, 0, 0) \
             RETURNING ino",
            (now, now, now),
        )
        .await
        .unwrap();
    let file_ino = *rows
        .next()
        .await
        .unwrap()
        .unwrap()
        .get_value(0)
        .unwrap()
        .as_integer()
        .unwrap();

    conn.execute(
        "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES ('child.txt', ?, ?)",
        (root_ino, file_ino),
    )
    .await
    .unwrap();

    // Insert a data chunk for the child inode.
    conn.execute(
        "INSERT INTO fs_data (ino, chunk_index, data) VALUES (?, 0, ?)",
        (file_ino, b"hello".to_vec()),
    )
    .await
    .unwrap();

    // Insert a separate child inode for a symlink.
    let mut rows2 = conn
        .query(
            "INSERT INTO fs_inode \
             (mode, nlink, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec) \
             VALUES (41471, 1, 0, 0, 0, ?, ?, ?, 0, 0, 0) \
             RETURNING ino",
            (now, now, now),
        )
        .await
        .unwrap();
    let link_ino = *rows2
        .next()
        .await
        .unwrap()
        .unwrap()
        .get_value(0)
        .unwrap()
        .as_integer()
        .unwrap();

    conn.execute(
        "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES ('link', ?, ?)",
        (root_ino, link_ino),
    )
    .await
    .unwrap();

    conn.execute(
        "INSERT INTO fs_symlink (ino, target) VALUES (?, '/etc/hosts')",
        (link_ino,),
    )
    .await
    .unwrap();

    // Destroy the volume.
    let destroyed = volume::destroy(&conn, "conv-X").await.unwrap();
    assert!(destroyed);

    // fs_data rows for file_ino must be gone.
    let mut data_rows = conn
        .query("SELECT 1 FROM fs_data WHERE ino = ?", (file_ino,))
        .await
        .unwrap();
    assert!(
        data_rows.next().await.unwrap().is_none(),
        "fs_data rows for destroyed inode must be deleted"
    );

    // fs_symlink rows for link_ino must be gone.
    let mut sym_rows = conn
        .query("SELECT 1 FROM fs_symlink WHERE ino = ?", (link_ino,))
        .await
        .unwrap();
    assert!(
        sym_rows.next().await.unwrap().is_none(),
        "fs_symlink rows for destroyed inode must be deleted"
    );

    // fs_inode rows for both child inodes must be gone.
    for ino in [file_ino, link_ino, root_ino] {
        let mut inode_rows = conn
            .query("SELECT 1 FROM fs_inode WHERE ino = ?", (ino,))
            .await
            .unwrap();
        assert!(
            inode_rows.next().await.unwrap().is_none(),
            "fs_inode row for ino={ino} must be deleted after destroy"
        );
    }
}

#[tokio::test]
async fn ensure_stamps_calling_process_uid_gid() {
    let conn = fresh_db().await;
    let root = volume::ensure(&conn, "conv-X").await.unwrap();
    let mut rows = conn
        .query("SELECT uid, gid FROM fs_inode WHERE ino = ?", (root,))
        .await
        .unwrap();
    let row = rows.next().await.unwrap().unwrap();
    let uid = *row.get_value(0).unwrap().as_integer().unwrap();
    let gid = *row.get_value(1).unwrap().as_integer().unwrap();
    let expected_uid = unsafe { libc::getuid() as i64 };
    let expected_gid = unsafe { libc::getgid() as i64 };
    assert_eq!(uid, expected_uid);
    assert_eq!(gid, expected_gid);
}
