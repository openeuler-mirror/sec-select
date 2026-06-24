/// Smoke test for `secafs serve api`.
///
/// Requires a running Postgres instance and a usable FUSE device.
/// Run with:
///
///   SECAFS_TEST_POSTGRES_URL=postgres://... \
///   cargo test --no-default-features -p secafs \
///     --test serve_api_smoke -- --ignored
///
/// All tests are marked `#[ignore]` so they are skipped in normal CI.
#[cfg(target_os = "linux")]
mod tests {
    use std::time::Duration;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::UnixStream;

    #[tokio::test]
    #[ignore]
    async fn serve_api_ping_responds() {
        let pg = std::env::var("SECAFS_TEST_POSTGRES_URL")
            .expect("SECAFS_TEST_POSTGRES_URL must be set");
        let tmp = tempfile::tempdir().unwrap();
        let socket = tmp.path().join("secafs.sock");
        let mount_root = tmp.path().join("mounts");

        let args = secafs::cmd::serve_api::ServeApiArgs {
            socket: socket.clone(),
            pg_url: pg,
            mount_root,
        };
        let handle = tokio::spawn(async move {
            let _ = secafs::cmd::serve_api::run(args).await;
        });

        // Wait for the socket to appear (up to 3 s).
        for _ in 0..30 {
            if socket.exists() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        assert!(socket.exists(), "socket did not appear within 3s");

        let mut stream = UnixStream::connect(&socket).await.unwrap();
        let req = b"{\"jsonrpc\":\"2.0\",\"method\":\"secafs.v1.ping\",\"params\":{},\"id\":1}\n";
        stream.write_all(req).await.unwrap();

        let (r, _) = stream.into_split();
        let mut reader = BufReader::new(r);
        let mut resp = String::new();
        reader.read_line(&mut resp).await.unwrap();

        assert!(
            resp.contains("\"result\""),
            "response missing result: {resp}"
        );
        assert!(
            resp.contains("pgConnected"),
            "response missing pgConnected: {resp}"
        );

        handle.abort();
    }
}
