use secafs_sdk::connection_pool::ConnectionPool;
use secafs_sdk::db::DbValue;
use secafs_sdk::filesystem::SecAFS;
use std::sync::Arc;

#[tokio::test]
async fn fs_volumes_table_exists_after_initialize() {
    let url = std::env::var("SECAFS_TEST_POSTGRES_URL")
        .expect("SECAFS_TEST_POSTGRES_URL required");

    let (client, connection) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
        .await
        .expect("failed to connect to postgres");

    tokio::spawn(async move {
        if let Err(e) = connection.await {
            eprintln!("postgres connection error: {}", e);
        }
    });

    let client = Arc::new(client);

    // Isolate from other tests: drop and recreate the public schema
    client
        .execute("DROP SCHEMA public CASCADE", &[])
        .await
        .unwrap();
    client
        .execute("CREATE SCHEMA public", &[])
        .await
        .unwrap();

    let pool = ConnectionPool::new(vec![Arc::clone(&client)]);
    let conn = pool.get_connection().await.unwrap();

    SecAFS::initialize_schema(&conn).await.unwrap();

    // Check that fs_volumes has the expected columns
    let mut rows = conn
        .query(
            "SELECT column_name FROM information_schema.columns \
             WHERE table_name = 'fs_volumes' AND table_schema = 'public' \
             ORDER BY column_name",
            (),
        )
        .await
        .unwrap();

    let mut cols: Vec<String> = Vec::new();
    while let Some(row) = rows.next().await.unwrap() {
        if let Ok(DbValue::Text(col)) = row.get_value(0) {
            cols.push(col);
        }
    }

    assert!(
        cols.contains(&"id".to_string()),
        "fs_volumes missing 'id'; found columns: {:?}",
        cols
    );
    assert!(
        cols.contains(&"root_ino".to_string()),
        "fs_volumes missing 'root_ino'; found columns: {:?}",
        cols
    );
    assert!(
        cols.contains(&"created_at".to_string()),
        "fs_volumes missing 'created_at'; found columns: {:?}",
        cols
    );

    // Check schema_version bumped to 0.5
    let mut version_rows = conn
        .query(
            "SELECT value FROM fs_config WHERE key = 'schema_version'",
            (),
        )
        .await
        .unwrap();

    let version_row = version_rows
        .next()
        .await
        .unwrap()
        .expect("schema_version row not found");
    let version = match version_row.get_value(0).unwrap() {
        DbValue::Text(v) => v,
        other => panic!("unexpected type for schema_version: {:?}", other),
    };

    assert_eq!(version, "0.5", "schema_version should be bumped to 0.5");
}
