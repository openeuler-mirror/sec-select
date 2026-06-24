//! Property test: arbitrary FS operation sequences round-trip through commit + restore.

use proptest::prelude::*;
use secafs_sdk::db::DbConn;

#[derive(Debug, Clone)]
enum Op {
    CreateFile { name: String },
    WriteChunk { name: String, chunk: i64, data: Vec<u8> },
    DeleteFile { name: String },
    SetKv { key: String, value: String },
    DeleteKv { key: String },
}

fn op_strategy() -> impl Strategy<Value = Op> {
    prop_oneof![
        "[a-z]{1,5}".prop_map(|n| Op::CreateFile { name: n }),
        ("[a-z]{1,5}", 0i64..4, prop::collection::vec(any::<u8>(), 0..32))
            .prop_map(|(n, c, d)| Op::WriteChunk { name: n, chunk: c, data: d }),
        "[a-z]{1,5}".prop_map(|n| Op::DeleteFile { name: n }),
        ("[a-z]{1,5}", "[a-z]{1,10}").prop_map(|(k, v)| Op::SetKv { key: k, value: v }),
        "[a-z]{1,5}".prop_map(|k| Op::DeleteKv { key: k }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig {
        cases: 12, // smaller default; PG round-trips dominate latency
        timeout: 60_000,
        .. ProptestConfig::default()
    })]

    #[test]
    fn restore_recovers_baseline(ops in prop::collection::vec(op_strategy(), 0..15)) {
        let Some(url) = std::env::var("SECAFS_TEST_DATABASE_URL").ok() else {
            return Ok(());
        };
        tokio_test::block_on(async {
            let opts = secafs_sdk::SecAFSOptions::with_postgres_url(url);
            let fs = secafs_sdk::SecAFS::open(opts).await.unwrap();
            let conn = fs.get_connection().await.unwrap();

            let vol = format!("proptest-{}", uuid::Uuid::new_v4());
            secafs_sdk::filesystem::volume::ensure(&conn, &vol).await.unwrap();
            secafs_sdk::snapshot::enable(&conn, &vol).await.unwrap();
            conn.execute(&format!("SET secafs.volume_id = '{vol}'"), ()).await.unwrap();

            let baseline = capture_state(&conn).await;
            let snap0 = secafs_sdk::snapshot::commit(&conn, &vol, Some("baseline")).await.unwrap();

            for op in &ops { apply_op(&conn, op).await; }
            let _ = secafs_sdk::snapshot::commit(&conn, &vol, Some("after")).await;

            secafs_sdk::snapshot::restore_to(&conn, &vol, snap0.snap_id).await.unwrap();
            let recovered = capture_state(&conn).await;

            // Cleanup: drop the volume so the global namespace doesn't pollute.
            secafs_sdk::filesystem::volume::destroy(&conn, &vol).await.unwrap();
            prop_assert_eq!(baseline, recovered);
            Ok(())
        })?;
    }
}

#[derive(Debug, PartialEq, Eq)]
struct Snapshot {
    inodes: Vec<(i64, i64)>,
    dentries: Vec<(String, i64)>,
    chunks: Vec<(i64, i64, Vec<u8>)>,
    kv: Vec<(String, String)>,
}

async fn capture_state(conn: &DbConn) -> Snapshot {
    let mut snap = Snapshot { inodes: vec![], dentries: vec![], chunks: vec![], kv: vec![] };
    let mut rows = conn.query("SELECT ino, size FROM fs_inode ORDER BY ino", ()).await.unwrap();
    while let Some(r) = rows.next().await.unwrap() {
        let ino = match r.get_value(0).unwrap() { secafs_sdk::db::DbValue::Integer(i) => i, _ => continue };
        let size = match r.get_value(1).unwrap() { secafs_sdk::db::DbValue::Integer(i) => i, _ => 0 };
        snap.inodes.push((ino, size));
    }
    let mut rows = conn.query("SELECT name, ino FROM fs_dentry ORDER BY id", ()).await.unwrap();
    while let Some(r) = rows.next().await.unwrap() {
        let name = match r.get_value(0).unwrap() { secafs_sdk::db::DbValue::Text(t) => t, _ => continue };
        let ino = match r.get_value(1).unwrap() { secafs_sdk::db::DbValue::Integer(i) => i, _ => continue };
        snap.dentries.push((name, ino));
    }
    let mut rows = conn.query("SELECT ino, chunk_index, data FROM fs_data ORDER BY ino, chunk_index", ()).await.unwrap();
    while let Some(r) = rows.next().await.unwrap() {
        let ino = match r.get_value(0).unwrap() { secafs_sdk::db::DbValue::Integer(i) => i, _ => continue };
        let chunk = match r.get_value(1).unwrap() { secafs_sdk::db::DbValue::Integer(i) => i, _ => continue };
        let data = match r.get_value(2).unwrap() { secafs_sdk::db::DbValue::Blob(b) => b, _ => vec![] };
        snap.chunks.push((ino, chunk, data));
    }
    let mut rows = conn.query("SELECT key, value FROM kv_store ORDER BY key", ()).await.unwrap();
    while let Some(r) = rows.next().await.unwrap() {
        let k = match r.get_value(0).unwrap() { secafs_sdk::db::DbValue::Text(t) => t, _ => continue };
        let v = match r.get_value(1).unwrap() { secafs_sdk::db::DbValue::Text(t) => t, _ => String::new() };
        snap.kv.push((k, v));
    }
    snap
}

async fn apply_op(conn: &DbConn, op: &Op) {
    match op {
        Op::CreateFile { name } => {
            // Insert inode, then dentry. Skip if dentry name already exists (UNIQUE constraint).
            let mut rows = conn.query(
                "INSERT INTO fs_inode (mode, atime, mtime, ctime) VALUES (33188, 0, 0, 0) RETURNING ino",
                (),
            ).await.unwrap();
            let row = rows.next().await.unwrap().unwrap();
            let ino: i64 = match row.get_value(0).unwrap() {
                secafs_sdk::db::DbValue::Integer(i) => i, _ => return,
            };
            let _ = conn.execute(
                "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES ($1, 1, $2)",
                (name.as_str(), ino),
            ).await;
        }
        Op::WriteChunk { name, chunk, data } => {
            if let Some(ino) = lookup_ino(conn, name).await {
                let args: Vec<secafs_sdk::db::DbValue> = vec![
                    secafs_sdk::db::DbValue::Integer(ino),
                    secafs_sdk::db::DbValue::Integer(*chunk),
                    secafs_sdk::db::DbValue::Blob(data.clone()),
                ];
                let _ = conn.execute(
                    "INSERT INTO fs_data (ino, chunk_index, data) VALUES ($1, $2, $3)
                     ON CONFLICT (ino, chunk_index) DO UPDATE SET data = EXCLUDED.data",
                    args,
                ).await;
            }
        }
        Op::DeleteFile { name } => {
            if let Some(ino) = lookup_ino(conn, name).await {
                let _ = conn.execute("DELETE FROM fs_data WHERE ino = $1", (ino,)).await;
                let _ = conn.execute("DELETE FROM fs_dentry WHERE ino = $1", (ino,)).await;
                let _ = conn.execute("DELETE FROM fs_inode WHERE ino = $1", (ino,)).await;
            }
        }
        Op::SetKv { key, value } => {
            let _ = conn.execute(
                "INSERT INTO kv_store (key, value) VALUES ($1, $2)
                 ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
                (key.as_str(), value.as_str()),
            ).await;
        }
        Op::DeleteKv { key } => {
            let _ = conn.execute("DELETE FROM kv_store WHERE key = $1", (key.as_str(),)).await;
        }
    }
}

async fn lookup_ino(conn: &DbConn, name: &str) -> Option<i64> {
    let mut rows = conn.query("SELECT ino FROM fs_dentry WHERE name = $1 LIMIT 1", (name,)).await.unwrap();
    rows.next().await.unwrap().and_then(|r| match r.get_value(0).unwrap() {
        secafs_sdk::db::DbValue::Integer(i) => Some(i), _ => None,
    })
}
