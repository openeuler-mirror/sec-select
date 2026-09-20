//! The pool that serves FUSE I/O has to survive the server closing its
//! connections — openGauss ships `session_timeout=10min`, and a restart or a
//! network blip does the same thing.
//!
//! Before `PoolHealer`, a closed connection stayed in the pool forever: every
//! op that round-robined onto it failed, and because only part of the pool was
//! dead the failure was *intermittent* (a pool of 2 with 1 dead connection
//! failed almost exactly half the reads).
//!
//! Requires a live PostgreSQL: set `SECAFS_TEST_POSTGRES_URL`. Skipped when
//! unset so the suite stays runnable without a database.

use std::sync::Arc;

use secafs_sdk::connection_pool::ConnectionPool;
use secafs_sdk::db::{DbValue, PoolHealer};
use secafs_sdk::DatabaseBackend;

fn test_url() -> Option<String> {
    std::env::var("SECAFS_TEST_POSTGRES_URL").ok()
}

async fn raw_client(url: &str) -> Arc<tokio_postgres::Client> {
    let (client, connection) = tokio_postgres::connect(url, tokio_postgres::NoTls)
        .await
        .expect("connect");
    tokio::spawn(async move {
        let _ = connection.await;
    });
    Arc::new(client)
}

/// The SDK row decoder only tries i64, so int4 results decode to Null —
/// these queries cast to bigint explicitly.
fn as_i64(v: &DbValue) -> i64 {
    *v.as_integer().expect("expected an integer column")
}

fn as_text(v: &DbValue) -> String {
    match v {
        DbValue::Text(s) => s.clone(),
        other => panic!("expected a text column, got {other:?}"),
    }
}

/// Backend pid of one specific pooled connection, so a test can kill exactly
/// that one and leave the rest of the pool alive.
async fn backend_pid(pool: &ConnectionPool, slot: usize) -> i32 {
    let conn = pool.get_connection_indexed(slot).await.expect("slot");
    let mut rows = conn.query("SELECT pg_backend_pid()::bigint", ()).await.expect("pid");
    let row = rows.next().await.expect("next").expect("row");
    as_i64(&row.get_value(0).expect("col")) as i32
}

/// Kill a backend from an independent connection — this is what the server
/// does to us on an idle timeout.
async fn terminate(url: &str, pid: i32) {
    let admin = raw_client(url).await;
    let _ = admin.query("SELECT pg_terminate_backend($1)", &[&pid]).await;
    // Let the driver task notice the closed socket and flip `is_closed()`.
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
}

async fn healing_pool(url: &str, size: usize, on_connect: Vec<String>) -> ConnectionPool {
    let mut clients = Vec::new();
    for _ in 0..size {
        let c = raw_client(url).await;
        for sql in &on_connect {
            c.batch_execute(sql).await.expect("prime");
        }
        clients.push(c);
    }
    let healer = Arc::new(PoolHealer::new(url.to_string(), on_connect));
    ConnectionPool::with_healer(clients, DatabaseBackend::Postgres, healer)
}

#[tokio::test]
async fn reconnects_a_connection_the_server_closed() {
    let Some(url) = test_url() else {
        eprintln!("SECAFS_TEST_POSTGRES_URL unset — skipping");
        return;
    };
    let pool = healing_pool(&url, 2, vec![]).await;

    // Kill one of the two backends: the exact shape of the production bug.
    let victim = backend_pid(&pool, 0).await;
    terminate(&url, victim).await;

    // Every checkout must answer. Without the healer this lands on the dead
    // slot roughly half the time and errors there.
    for i in 0..20 {
        let conn = pool.get_connection().await.expect("checkout");
        let mut rows = conn
            .query("SELECT 1::bigint", ())
            .await
            .unwrap_or_else(|e| panic!("op {i} failed after self-heal: {e}"));
        let row = rows.next().await.expect("next").expect("row");
        assert_eq!(as_i64(&row.get_value(0).expect("col")), 1);
    }
}

#[tokio::test]
async fn replays_gucs_on_a_reconnected_connection() {
    let Some(url) = test_url() else {
        eprintln!("SECAFS_TEST_POSTGRES_URL unset — skipping");
        return;
    };
    // A mount pool carries these. A reconnect that forgets them makes the undo
    // triggers fail on the next *write* — a nastier failure than the read
    // errors that motivated this fix.
    let on_connect = vec![
        "SET secafs.volume_id = 'vol-under-test'".to_string(),
        "SET secafs.suppress_undo = 'false'".to_string(),
    ];
    let pool = healing_pool(&url, 1, on_connect).await;

    let victim = backend_pid(&pool, 0).await;
    terminate(&url, victim).await;

    let conn = pool.get_connection().await.expect("checkout after kill");
    let mut rows = conn
        .query("SELECT current_setting('secafs.volume_id')", ())
        .await
        .expect("GUC must be readable on the fresh connection");
    let row = rows.next().await.expect("next").expect("row");
    assert_eq!(
        as_text(&row.get_value(0).expect("col")),
        "vol-under-test",
        "reconnect must replay the per-volume GUCs"
    );
}

#[tokio::test]
async fn pool_without_healer_keeps_the_old_behaviour() {
    let Some(url) = test_url() else {
        eprintln!("SECAFS_TEST_POSTGRES_URL unset — skipping");
        return;
    };
    // Tests and one-shot tools build pools this way; they must not silently
    // acquire reconnect behaviour (or a DSN they never supplied).
    let pool =
        ConnectionPool::with_backend(vec![raw_client(&url).await], DatabaseBackend::Postgres);
    let victim = backend_pid(&pool, 0).await;
    terminate(&url, victim).await;

    let conn = pool.get_connection().await.expect("still hands out a conn");
    assert!(conn.is_closed(), "dead connection is handed back as before");
    assert!(
        conn.query("SELECT 1::bigint", ()).await.is_err(),
        "and it fails"
    );
}

/// The reconnect happens while holding the slot's write lock, and FUSE drives
/// many concurrent ops. Two things have to hold: the healer must never reach
/// back into the pool (that would self-deadlock), and the callers queued
/// behind the one that reconnects must reuse its connection instead of each
/// opening another.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrent_callers_on_a_dead_slot_reconnect_once() {
    let Some(url) = test_url() else {
        eprintln!("SECAFS_TEST_POSTGRES_URL unset — skipping");
        return;
    };
    // A single slot, so every caller contends for the same lock.
    let pool = healing_pool(&url, 1, vec![]).await;
    let victim = backend_pid(&pool, 0).await;
    terminate(&url, victim).await;

    let mut set = tokio::task::JoinSet::new();
    for _ in 0..16 {
        let p = pool.clone();
        set.spawn(async move {
            let conn = p.get_connection().await.expect("checkout");
            let mut rows = conn
                .query("SELECT pg_backend_pid()::bigint", ())
                .await
                .expect("query");
            let row = rows.next().await.expect("next").expect("row");
            as_i64(&row.get_value(0).expect("col"))
        });
    }

    // A deadlock would hang the whole suite — bound it and fail instead.
    let mut pids = Vec::new();
    let finished = tokio::time::timeout(std::time::Duration::from_secs(20), async {
        while let Some(r) = set.join_next().await {
            pids.push(r.expect("task panicked"));
        }
    })
    .await;
    assert!(finished.is_ok(), "concurrent checkouts deadlocked on the slot lock");

    assert_eq!(pids.len(), 16, "every caller must get served");
    let first = pids[0];
    assert!(
        pids.iter().all(|p| *p == first),
        "double-check under the write lock should mean ONE reconnect shared by \
         all callers; got distinct backends: {pids:?}"
    );
    assert_ne!(first as i32, victim, "must be a new backend, not the killed one");
}
