//! DDL for v0.6 snapshot tables.
//!
//! All tables are additive — no existing v0.5 tables are altered.

/// Returns the DDL statements (in order) that create the v0.6 snapshot
/// tables. Each statement is idempotent (`IF NOT EXISTS`).
pub fn ddl_statements() -> Vec<&'static str> {
    vec![
        // Per-volume rollback state.
        "CREATE TABLE IF NOT EXISTS fs_volume_state (
            volume_id        TEXT PRIMARY KEY REFERENCES fs_volumes(id) ON DELETE CASCADE,
            rollback_enabled BOOLEAN NOT NULL DEFAULT FALSE,
            current_snap_id  BIGINT  NOT NULL DEFAULT 1
        )",
        // Committed snapshot points.
        "CREATE TABLE IF NOT EXISTS fs_snapshots (
            volume_id    TEXT   NOT NULL REFERENCES fs_volumes(id) ON DELETE CASCADE,
            snap_id      BIGINT NOT NULL,
            committed_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
            label        TEXT,
            PRIMARY KEY (volume_id, snap_id)
        )",
        "CREATE UNIQUE INDEX IF NOT EXISTS fs_snapshots_label_uniq
            ON fs_snapshots(volume_id, label) WHERE label IS NOT NULL",
        // fs_inode_undo
        "CREATE TABLE IF NOT EXISTS fs_inode_undo (
            undo_id    BIGSERIAL PRIMARY KEY,
            volume_id  TEXT    NOT NULL,
            snap_id    BIGINT  NOT NULL,
            op         CHAR(1) NOT NULL,
            ino        BIGINT  NOT NULL,
            mode       BIGINT,
            nlink      BIGINT,
            uid        BIGINT,
            gid        BIGINT,
            size       BIGINT,
            atime      BIGINT,
            mtime      BIGINT,
            ctime      BIGINT,
            rdev       BIGINT,
            atime_nsec BIGINT,
            mtime_nsec BIGINT,
            ctime_nsec BIGINT
        )",
        "CREATE INDEX IF NOT EXISTS idx_fs_inode_undo_replay
            ON fs_inode_undo(volume_id, snap_id DESC, undo_id DESC)",
        // fs_dentry_undo
        "CREATE TABLE IF NOT EXISTS fs_dentry_undo (
            undo_id    BIGSERIAL PRIMARY KEY,
            volume_id  TEXT    NOT NULL,
            snap_id    BIGINT  NOT NULL,
            op         CHAR(1) NOT NULL,
            id         BIGINT  NOT NULL,
            name       TEXT,
            parent_ino BIGINT,
            ino        BIGINT
        )",
        "CREATE INDEX IF NOT EXISTS idx_fs_dentry_undo_replay
            ON fs_dentry_undo(volume_id, snap_id DESC, undo_id DESC)",
        // fs_data_undo (BYTEA payload, no JSONB).
        "CREATE TABLE IF NOT EXISTS fs_data_undo (
            undo_id     BIGSERIAL PRIMARY KEY,
            volume_id   TEXT    NOT NULL,
            snap_id     BIGINT  NOT NULL,
            op          CHAR(1) NOT NULL,
            ino         BIGINT  NOT NULL,
            chunk_index BIGINT  NOT NULL,
            data        BYTEA
        )",
        "CREATE INDEX IF NOT EXISTS idx_fs_data_undo_replay
            ON fs_data_undo(volume_id, snap_id DESC, undo_id DESC)",
        // fs_symlink_undo
        "CREATE TABLE IF NOT EXISTS fs_symlink_undo (
            undo_id    BIGSERIAL PRIMARY KEY,
            volume_id  TEXT    NOT NULL,
            snap_id    BIGINT  NOT NULL,
            op         CHAR(1) NOT NULL,
            ino        BIGINT  NOT NULL,
            target     TEXT
        )",
        "CREATE INDEX IF NOT EXISTS idx_fs_symlink_undo_replay
            ON fs_symlink_undo(volume_id, snap_id DESC, undo_id DESC)",
        // kv_store_undo
        "CREATE TABLE IF NOT EXISTS kv_store_undo (
            undo_id    BIGSERIAL PRIMARY KEY,
            volume_id  TEXT    NOT NULL,
            snap_id    BIGINT  NOT NULL,
            op         CHAR(1) NOT NULL,
            key        TEXT    NOT NULL,
            value      TEXT,
            created_at BIGINT,
            updated_at BIGINT
        )",
        "CREATE INDEX IF NOT EXISTS idx_kv_store_undo_replay
            ON kv_store_undo(volume_id, snap_id DESC, undo_id DESC)",
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_statements_are_nonempty() {
        let stmts = ddl_statements();
        assert!(stmts.len() >= 12);
        assert!(stmts.iter().all(|s| s.contains("IF NOT EXISTS") || s.contains("UNIQUE INDEX")));
    }
}
