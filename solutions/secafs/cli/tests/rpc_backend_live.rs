/// Live integration tests for `FuseMountBackend`.
///
/// These tests require:
///   - A running Postgres instance with `SECAFS_TEST_POSTGRES_URL` set.
///   - A usable FUSE device (`/dev/fuse` readable, `fusermount` available).
///   - Linux only (FUSE is not supported on macOS).
///
/// Run with:
///   SECAFS_TEST_POSTGRES_URL=postgres://... \
///   cargo test --no-default-features -p secafs \
///     --test rpc_backend_live -- --ignored
///
/// All tests are marked `#[ignore]` so they are skipped in normal CI.
#[cfg(target_os = "linux")]
mod tests {
    use std::path::Path;
    use std::sync::Arc;

    use secafs::rpc::backend::FuseMountBackend;
    use secafs::rpc::methods::MountBackend;

    /// Return `true` when FUSE is likely usable in the current environment.
    ///
    /// Checks that `/dev/fuse` is accessible and `fusermount` is on `$PATH`.
    /// This avoids a confusing kernel error when the test runs in a container
    /// or CI sandbox without FUSE support.
    fn fuse_available() -> bool {
        // /dev/fuse must exist and be accessible.
        if !Path::new("/dev/fuse").exists() {
            eprintln!("[skip] /dev/fuse not present");
            return false;
        }
        // fusermount must be reachable.
        match std::process::Command::new("fusermount").arg("--version").output() {
            Ok(out) if out.status.success() || out.status.code() == Some(1) => {}
            _ => {
                eprintln!("[skip] fusermount not available");
                return false;
            }
        }
        true
    }

    /// Read `SECAFS_TEST_POSTGRES_URL` or skip with a message.
    fn pg_url() -> Option<String> {
        match std::env::var("SECAFS_TEST_POSTGRES_URL") {
            Ok(u) if !u.is_empty() => Some(u),
            _ => {
                eprintln!("[skip] SECAFS_TEST_POSTGRES_URL not set");
                None
            }
        }
    }

    /// Check that `path` is a mount point by looking it up in `/proc/mounts`.
    fn is_mountpoint(path: &Path) -> bool {
        let target = path.to_string_lossy();
        std::fs::read_to_string("/proc/mounts")
            .map(|contents| contents.lines().any(|l| l.split_whitespace().nth(1) == Some(&*target)))
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Test: full mount → file I/O → unmount → destroy lifecycle
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn mount_write_read_unmount_destroy() {
        let Some(url) = pg_url() else { return };
        if !fuse_available() {
            return;
        }

        let backend = FuseMountBackend::new(&url, 2)
            .await
            .expect("FuseMountBackend::new failed");

        let tmpdir = tempfile::tempdir().expect("tempdir");
        let mnt = tmpdir.path().to_path_buf();

        // --- mount ---
        backend
            .mount("test-rpc-live", &mnt)
            .await
            .expect("mount failed");

        assert!(
            is_mountpoint(&mnt),
            "expected {mnt:?} to be a mount point after mount()"
        );

        // --- write a file through the FUSE mount ---
        let test_file = mnt.join("hello.txt");
        std::fs::write(&test_file, b"hello secafs").expect("write to mount failed");

        // --- read it back ---
        let contents = std::fs::read(&test_file).expect("read from mount failed");
        assert_eq!(contents, b"hello secafs", "round-trip content mismatch");

        // --- unmount ---
        backend
            .unmount("test-rpc-live")
            .await
            .expect("unmount failed");

        // Give the kernel a moment to process the unmount.
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        assert!(
            !is_mountpoint(&mnt),
            "expected {mnt:?} to no longer be a mount point after unmount()"
        );

        // --- destroy: DB rows should be gone ---
        backend
            .destroy("test-rpc-live")
            .await
            .expect("destroy failed");

        // Verify via a fresh connection that fs_volumes row is gone.
        let (client, conn_future) = tokio_postgres::connect(&url, tokio_postgres::NoTls)
            .await
            .expect("verify connect failed");
        tokio::spawn(conn_future);
        let rows = client
            .query(
                "SELECT root_ino FROM fs_volumes WHERE id = $1",
                &[&"test-rpc-live"],
            )
            .await
            .expect("query failed");
        assert!(rows.is_empty(), "fs_volumes row must be gone after destroy");
    }

    // -----------------------------------------------------------------------
    // Test: unmount of a non-existent id is a no-op
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn unmount_unknown_id_is_noop() {
        let Some(url) = pg_url() else { return };
        if !fuse_available() {
            return;
        }

        let backend = FuseMountBackend::new(&url, 1)
            .await
            .expect("FuseMountBackend::new failed");

        // Should not error.
        backend
            .unmount("no-such-volume")
            .await
            .expect("unmount of unknown id must be a no-op");
    }

    // -----------------------------------------------------------------------
    // Test: two concurrent mounts at different paths for different volumes
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore]
    async fn two_concurrent_mounts() {
        let Some(url) = pg_url() else { return };
        if !fuse_available() {
            return;
        }

        let backend: Arc<dyn MountBackend> = FuseMountBackend::new(&url, 4)
            .await
            .expect("FuseMountBackend::new failed");

        let tmpdir_a = tempfile::tempdir().expect("tmpdir a");
        let tmpdir_b = tempfile::tempdir().expect("tmpdir b");

        backend
            .mount("live-vol-A", tmpdir_a.path())
            .await
            .expect("mount A failed");
        backend
            .mount("live-vol-B", tmpdir_b.path())
            .await
            .expect("mount B failed");

        assert!(is_mountpoint(tmpdir_a.path()), "A not mounted");
        assert!(is_mountpoint(tmpdir_b.path()), "B not mounted");

        // Write different content to each mount.
        std::fs::write(tmpdir_a.path().join("a.txt"), b"volume-A").expect("write A");
        std::fs::write(tmpdir_b.path().join("b.txt"), b"volume-B").expect("write B");

        assert_eq!(
            std::fs::read(tmpdir_a.path().join("a.txt")).expect("read A"),
            b"volume-A"
        );
        assert_eq!(
            std::fs::read(tmpdir_b.path().join("b.txt")).expect("read B"),
            b"volume-B"
        );

        backend.unmount("live-vol-A").await.expect("unmount A");
        backend.unmount("live-vol-B").await.expect("unmount B");

        backend.destroy("live-vol-A").await.expect("destroy A");
        backend.destroy("live-vol-B").await.expect("destroy B");
    }
}
