use crate::db::{DbConn, DbTransaction, TransactionBehavior};
use crate::error::Result;
use std::time::{SystemTime, UNIX_EPOCH};

use super::DEFAULT_DIR_MODE;

/// Ensure a volume exists for the given id. Idempotent.
///
/// On first call, creates an `fs_inode` row with directory mode (`DEFAULT_DIR_MODE`),
/// a `fs_dentry` entry under the global root (parent_ino=1) named `id`, and an
/// `fs_volumes` row linking `id → new_ino`.
///
/// Subsequent calls with the same id return the existing root_ino unchanged.
pub async fn ensure(conn: &DbConn, id: &str) -> Result<i64> {
    // Short-circuit if the volume already exists.
    let mut rows = conn
        .query("SELECT root_ino FROM fs_volumes WHERE id = ?", (id,))
        .await?;

    if let Some(row) = rows.next().await? {
        let ino = row.get_value(0)?;
        return Ok(*ino.as_integer().expect("root_ino is an integer"));
    }

    // Create the root inode, dentry, and volumes row atomically so that a crash
    // between sub-steps cannot leave a dangling fs_inode without a corresponding
    // fs_volumes entry.
    let tx = DbTransaction::new_unchecked(conn, TransactionBehavior::Deferred).await?;

    let dur = SystemTime::now().duration_since(UNIX_EPOCH)?;
    let now_secs = dur.as_secs() as i64;
    let now_nsec = dur.subsec_nanos() as i64;

    // Stamp the calling process's uid/gid so that FUSE `default_permissions`
    // allows the invoking user to write to the volume root.
    let (uid, gid) = unsafe { (libc::getuid() as i64, libc::getgid() as i64) };

    let mut ino_rows = conn
        .query(
            "INSERT INTO fs_inode \
             (mode, nlink, uid, gid, size, atime, mtime, ctime, atime_nsec, mtime_nsec, ctime_nsec) \
             VALUES (?, 2, ?, ?, 0, ?, ?, ?, ?, ?, ?) \
             RETURNING ino",
            (
                DEFAULT_DIR_MODE as i64,
                uid,
                gid,
                now_secs,
                now_secs,
                now_secs,
                now_nsec,
                now_nsec,
                now_nsec,
            ),
        )
        .await?;

    let ino_row = ino_rows
        .next()
        .await?
        .expect("INSERT ... RETURNING ino must return a row");
    let root_ino = *ino_row
        .get_value(0)?
        .as_integer()
        .expect("ino is an integer");

    // Add a dentry under the global root (ino=1) so path resolution works.
    conn.execute(
        "INSERT INTO fs_dentry (name, parent_ino, ino) VALUES (?, 1, ?)",
        (id, root_ino),
    )
    .await?;

    // Register the volume.
    conn.execute(
        "INSERT INTO fs_volumes (id, root_ino) VALUES (?, ?)",
        (id, root_ino),
    )
    .await?;

    tx.commit().await?;

    Ok(root_ino)
}

/// Remove a volume and its entire tree. Idempotent.
///
/// Deletes the `fs_volumes` row, then all dentries and inodes rooted at the
/// volume's `root_ino`, then the root `fs_inode` itself.
///
/// Returns `Ok(true)` if a volume was destroyed, `Ok(false)` if no such id existed.
pub async fn destroy(conn: &DbConn, id: &str) -> Result<bool> {
    // Look up the root_ino so we know what to clean up.
    let mut rows = conn
        .query("SELECT root_ino FROM fs_volumes WHERE id = ?", (id,))
        .await?;

    let root_ino = match rows.next().await? {
        None => return Ok(false),
        Some(row) => *row.get_value(0)?.as_integer().expect("root_ino is an integer"),
    };

    // Remove the fs_volumes row first (FK references fs_inode; must go before inode delete).
    conn.execute("DELETE FROM fs_volumes WHERE id = ?", (id,))
        .await?;

    // Delete the dentry under the global root that points to this volume root.
    conn.execute(
        "DELETE FROM fs_dentry WHERE parent_ino = 1 AND ino = ?",
        (root_ino,),
    )
    .await?;

    // All four remaining deletes (fs_data, fs_symlink, fs_dentry, fs_inode) use a
    // recursive CTE that walks fs_dentry.  They must all run while the dentry rows
    // for descendant inodes are still present; fs_dentry is removed last.
    // Cast the parameter to bigint so Postgres can match it against the ino column.

    // Delete data chunks for all inodes in the subtree.
    conn.execute(
        "WITH RECURSIVE subtree(ino) AS (
             SELECT ?::bigint \
             UNION ALL \
             SELECT d.ino FROM fs_dentry d JOIN subtree s ON d.parent_ino = s.ino
         )
         DELETE FROM fs_data WHERE ino IN (SELECT ino FROM subtree)",
        (root_ino,),
    )
    .await?;

    // Delete symlink targets for all inodes in the subtree.
    conn.execute(
        "WITH RECURSIVE subtree(ino) AS (
             SELECT ?::bigint \
             UNION ALL \
             SELECT d.ino FROM fs_dentry d JOIN subtree s ON d.parent_ino = s.ino
         )
         DELETE FROM fs_symlink WHERE ino IN (SELECT ino FROM subtree)",
        (root_ino,),
    )
    .await?;

    // Delete all inodes in the subtree (while dentries still exist for traversal).
    conn.execute(
        "WITH RECURSIVE subtree(ino) AS (
             SELECT ?::bigint \
             UNION ALL \
             SELECT d.ino FROM fs_dentry d JOIN subtree s ON d.parent_ino = s.ino
         )
         DELETE FROM fs_inode WHERE ino IN (SELECT ino FROM subtree)",
        (root_ino,),
    )
    .await?;

    // Remove dentries last (their FK to fs_inode is gone; dentry rows are plain data now).
    conn.execute(
        "WITH RECURSIVE subtree(ino) AS (
             SELECT ?::bigint \
             UNION ALL \
             SELECT d.ino FROM fs_dentry d JOIN subtree s ON d.parent_ino = s.ino
         )
         DELETE FROM fs_dentry WHERE ino IN (SELECT ino FROM subtree)
                                  OR parent_ino IN (SELECT ino FROM subtree)",
        (root_ino,),
    )
    .await?;

    Ok(true)
}
