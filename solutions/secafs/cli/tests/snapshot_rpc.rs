/// Smoke test for the secafs.v1.snapshot.* RPC methods.
///
/// Requires SECAFS_TEST_DATABASE_URL to be set (skipped otherwise).
/// Run with:
///   SECAFS_TEST_DATABASE_URL="postgres://secafs:secafs@localhost:5433/secafs" \
///   cargo test snapshot_rpc -- --nocapture
///
/// (from secafs/cli/)

#[tokio::test]
async fn snapshot_rpc_round_trip() {
    let url = match std::env::var("SECAFS_TEST_DATABASE_URL").ok() {
        Some(u) if !u.is_empty() => u,
        _ => {
            eprintln!("[skip] SECAFS_TEST_DATABASE_URL not set");
            return;
        }
    };

    // Reset the DB to a clean state.
    let (client, conn_future) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("connect for reset failed");
    tokio::spawn(async move { let _ = conn_future.await; });
    client
        .batch_execute("DROP SCHEMA public CASCADE; CREATE SCHEMA public;")
        .await
        .expect("schema reset failed");
    drop(client);

    // Bootstrap schema (initializes all tables including v0.6 snapshot tables).
    secafs_sdk::SecAFS::open(
        secafs_sdk::SecAFSOptions::with_postgres_url(url.clone()),
    )
    .await
    .expect("SecAFS::open (schema bootstrap) failed");

    // Build a FuseMountBackend backed by the same DB.
    use secafs::rpc::backend::FuseMountBackend;
    use secafs::rpc::methods::{dispatch, State};
    use std::sync::Arc;
    use tokio::sync::Mutex;
    use std::collections::HashMap;

    let backend = FuseMountBackend::new(&url, 4)
        .await
        .expect("FuseMountBackend::new failed");

    let state = Arc::new(State {
        backend: backend.clone() as Arc<dyn secafs::rpc::methods::MountBackend>,
        mount_root: std::env::temp_dir().join("rpc-smoke"),
        mounts: Mutex::new(HashMap::new()),
    });

    let conv = "rpc-smoke";

    // Ensure the fs_volumes row exists (snapshot::enable requires it for the FK).
    {
        let conn = state.backend.get_connection().await
            .expect("get_connection failed");
        secafs_sdk::filesystem::volume::ensure(&conn, conv)
            .await
            .expect("volume::ensure failed");
    }

    // --- enable ---
    let r = dispatch(
        &state,
        "secafs.v1.snapshot.enable",
        serde_json::json!({"conversationId": conv}),
    )
    .await
    .expect("snapshot.enable failed");
    assert_eq!(r["enabled"], serde_json::json!(true), "enable: rollback_enabled mismatch");

    // --- commit (first snapshot) ---
    let r = dispatch(
        &state,
        "secafs.v1.snapshot.commit",
        serde_json::json!({"conversationId": conv, "label": "first"}),
    )
    .await
    .expect("snapshot.commit failed");
    let snap1 = r["snapId"].as_i64().expect("snapId should be i64");
    assert_eq!(snap1, 1, "first snap_id should be 1");

    // --- list ---
    let r = dispatch(
        &state,
        "secafs.v1.snapshot.list",
        serde_json::json!({"conversationId": conv}),
    )
    .await
    .expect("snapshot.list failed");
    let snaps = r["snapshots"].as_array().expect("snapshots should be array");
    assert_eq!(snaps.len(), 1, "should have exactly 1 snapshot after first commit");

    // --- restore ---
    let r = dispatch(
        &state,
        "secafs.v1.snapshot.restore",
        serde_json::json!({"conversationId": conv, "snapId": snap1}),
    )
    .await
    .expect("snapshot.restore failed");
    assert_eq!(r["restored"], serde_json::json!(true), "restore: restored should be true");

    // --- disable ---
    let r = dispatch(
        &state,
        "secafs.v1.snapshot.disable",
        serde_json::json!({"conversationId": conv}),
    )
    .await
    .expect("snapshot.disable failed");
    assert_eq!(r["disabled"], serde_json::json!(true), "disable: disabled should be true");
}
