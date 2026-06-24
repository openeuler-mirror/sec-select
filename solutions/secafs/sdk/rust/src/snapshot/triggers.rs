//! Trigger functions and bindings that capture before-images into the typed
//! undo tables. All functions short-circuit when:
//!  - The session-local GUC `secafs.volume_id` is unset (admin/migration paths).
//!  - The session-local GUC `secafs.suppress_undo` is `true` (restore replay).
//!  - The volume's `fs_volume_state.rollback_enabled` is `false` (default).

/// Returns the DDL statements (in order) that create trigger functions
/// and bindings. Idempotent via `CREATE OR REPLACE FUNCTION` and
/// `DROP TRIGGER IF EXISTS` + `CREATE TRIGGER`.
pub fn ddl_statements() -> Vec<&'static str> {
    vec![
        FN_FS_INODE,
        BIND_FS_INODE,
        FN_FS_DENTRY,
        BIND_FS_DENTRY,
        FN_FS_DATA,
        BIND_FS_DATA,
        FN_FS_SYMLINK,
        BIND_FS_SYMLINK,
        FN_KV_STORE,
        BIND_KV_STORE,
    ]
}

const FN_FS_INODE: &str = r#"
CREATE OR REPLACE FUNCTION fs_inode_capture_undo() RETURNS trigger AS $$
DECLARE
  vid  text := current_setting('secafs.volume_id',     true);
  supp text := current_setting('secafs.suppress_undo', true);
  state record;
BEGIN
  IF vid IS NULL OR vid = '' THEN RETURN NULL; END IF;
  IF supp = 'true' THEN RETURN NULL; END IF;
  -- Guard against zero-row `SELECT INTO`: OpenGauss 6.0 raises
  -- `query returned no rows when process INTO` in its default plpgsql mode,
  -- which would abort the user's INSERT/UPDATE on the triggered table when
  -- the volume has not yet been registered (no `fs_volume_state` row, e.g.
  -- before `snapshot.enable`). PG silently leaves `state` NULL and the
  -- following `IF NOT FOUND` would handle it, but OG never gets there.
  -- The `IF NOT EXISTS` short-circuit works on both backends.
  IF NOT EXISTS (SELECT 1 FROM fs_volume_state WHERE volume_id = vid) THEN
    RETURN NULL;
  END IF;
  SELECT rollback_enabled, current_snap_id INTO state
    FROM fs_volume_state WHERE volume_id = vid;
  IF NOT state.rollback_enabled THEN RETURN NULL; END IF;

  IF TG_OP = 'INSERT' THEN
    INSERT INTO fs_inode_undo(volume_id, snap_id, op, ino)
      VALUES (vid, state.current_snap_id, 'I', NEW.ino);
  ELSIF TG_OP = 'UPDATE' THEN
    INSERT INTO fs_inode_undo(volume_id, snap_id, op, ino, mode, nlink, uid, gid, size,
                              atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec)
      VALUES (vid, state.current_snap_id, 'U', OLD.ino, OLD.mode, OLD.nlink, OLD.uid, OLD.gid,
              OLD.size, OLD.atime, OLD.mtime, OLD.ctime, OLD.rdev,
              OLD.atime_nsec, OLD.mtime_nsec, OLD.ctime_nsec);
  ELSIF TG_OP = 'DELETE' THEN
    INSERT INTO fs_inode_undo(volume_id, snap_id, op, ino, mode, nlink, uid, gid, size,
                              atime, mtime, ctime, rdev, atime_nsec, mtime_nsec, ctime_nsec)
      VALUES (vid, state.current_snap_id, 'D', OLD.ino, OLD.mode, OLD.nlink, OLD.uid, OLD.gid,
              OLD.size, OLD.atime, OLD.mtime, OLD.ctime, OLD.rdev,
              OLD.atime_nsec, OLD.mtime_nsec, OLD.ctime_nsec);
  END IF;
  RETURN NULL;
END $$ LANGUAGE plpgsql;
"#;

const BIND_FS_INODE: &str = r#"
DROP TRIGGER IF EXISTS tr_fs_inode_undo ON fs_inode;
CREATE TRIGGER tr_fs_inode_undo AFTER INSERT OR UPDATE OR DELETE ON fs_inode
  FOR EACH ROW EXECUTE FUNCTION fs_inode_capture_undo();
"#;

const FN_FS_DENTRY: &str = r#"
CREATE OR REPLACE FUNCTION fs_dentry_capture_undo() RETURNS trigger AS $$
DECLARE
  vid  text := current_setting('secafs.volume_id',     true);
  supp text := current_setting('secafs.suppress_undo', true);
  state record;
BEGIN
  IF vid IS NULL OR vid = '' THEN RETURN NULL; END IF;
  IF supp = 'true' THEN RETURN NULL; END IF;
  -- Guard against zero-row `SELECT INTO`: OpenGauss 6.0 raises
  -- `query returned no rows when process INTO` in its default plpgsql mode,
  -- which would abort the user's INSERT/UPDATE on the triggered table when
  -- the volume has not yet been registered (no `fs_volume_state` row, e.g.
  -- before `snapshot.enable`). PG silently leaves `state` NULL and the
  -- following `IF NOT FOUND` would handle it, but OG never gets there.
  -- The `IF NOT EXISTS` short-circuit works on both backends.
  IF NOT EXISTS (SELECT 1 FROM fs_volume_state WHERE volume_id = vid) THEN
    RETURN NULL;
  END IF;
  SELECT rollback_enabled, current_snap_id INTO state
    FROM fs_volume_state WHERE volume_id = vid;
  IF NOT state.rollback_enabled THEN RETURN NULL; END IF;

  IF TG_OP = 'INSERT' THEN
    INSERT INTO fs_dentry_undo(volume_id, snap_id, op, id)
      VALUES (vid, state.current_snap_id, 'I', NEW.id);
  ELSIF TG_OP = 'UPDATE' THEN
    INSERT INTO fs_dentry_undo(volume_id, snap_id, op, id, name, parent_ino, ino)
      VALUES (vid, state.current_snap_id, 'U', OLD.id, OLD.name, OLD.parent_ino, OLD.ino);
  ELSIF TG_OP = 'DELETE' THEN
    INSERT INTO fs_dentry_undo(volume_id, snap_id, op, id, name, parent_ino, ino)
      VALUES (vid, state.current_snap_id, 'D', OLD.id, OLD.name, OLD.parent_ino, OLD.ino);
  END IF;
  RETURN NULL;
END $$ LANGUAGE plpgsql;
"#;

const BIND_FS_DENTRY: &str = r#"
DROP TRIGGER IF EXISTS tr_fs_dentry_undo ON fs_dentry;
CREATE TRIGGER tr_fs_dentry_undo AFTER INSERT OR UPDATE OR DELETE ON fs_dentry
  FOR EACH ROW EXECUTE FUNCTION fs_dentry_capture_undo();
"#;

const FN_FS_DATA: &str = r#"
CREATE OR REPLACE FUNCTION fs_data_capture_undo() RETURNS trigger AS $$
DECLARE
  vid  text := current_setting('secafs.volume_id',     true);
  supp text := current_setting('secafs.suppress_undo', true);
  state record;
BEGIN
  IF vid IS NULL OR vid = '' THEN RETURN NULL; END IF;
  IF supp = 'true' THEN RETURN NULL; END IF;
  -- Guard against zero-row `SELECT INTO`: OpenGauss 6.0 raises
  -- `query returned no rows when process INTO` in its default plpgsql mode,
  -- which would abort the user's INSERT/UPDATE on the triggered table when
  -- the volume has not yet been registered (no `fs_volume_state` row, e.g.
  -- before `snapshot.enable`). PG silently leaves `state` NULL and the
  -- following `IF NOT FOUND` would handle it, but OG never gets there.
  -- The `IF NOT EXISTS` short-circuit works on both backends.
  IF NOT EXISTS (SELECT 1 FROM fs_volume_state WHERE volume_id = vid) THEN
    RETURN NULL;
  END IF;
  SELECT rollback_enabled, current_snap_id INTO state
    FROM fs_volume_state WHERE volume_id = vid;
  IF NOT state.rollback_enabled THEN RETURN NULL; END IF;

  IF TG_OP = 'INSERT' THEN
    INSERT INTO fs_data_undo(volume_id, snap_id, op, ino, chunk_index, data)
      VALUES (vid, state.current_snap_id, 'I', NEW.ino, NEW.chunk_index, NULL);
  ELSIF TG_OP = 'UPDATE' THEN
    INSERT INTO fs_data_undo(volume_id, snap_id, op, ino, chunk_index, data)
      VALUES (vid, state.current_snap_id, 'U', OLD.ino, OLD.chunk_index, OLD.data);
  ELSIF TG_OP = 'DELETE' THEN
    INSERT INTO fs_data_undo(volume_id, snap_id, op, ino, chunk_index, data)
      VALUES (vid, state.current_snap_id, 'D', OLD.ino, OLD.chunk_index, OLD.data);
  END IF;
  RETURN NULL;
END $$ LANGUAGE plpgsql;
"#;

const BIND_FS_DATA: &str = r#"
DROP TRIGGER IF EXISTS tr_fs_data_undo ON fs_data;
CREATE TRIGGER tr_fs_data_undo AFTER INSERT OR UPDATE OR DELETE ON fs_data
  FOR EACH ROW EXECUTE FUNCTION fs_data_capture_undo();
"#;

const FN_FS_SYMLINK: &str = r#"
CREATE OR REPLACE FUNCTION fs_symlink_capture_undo() RETURNS trigger AS $$
DECLARE
  vid  text := current_setting('secafs.volume_id',     true);
  supp text := current_setting('secafs.suppress_undo', true);
  state record;
BEGIN
  IF vid IS NULL OR vid = '' THEN RETURN NULL; END IF;
  IF supp = 'true' THEN RETURN NULL; END IF;
  -- Guard against zero-row `SELECT INTO`: OpenGauss 6.0 raises
  -- `query returned no rows when process INTO` in its default plpgsql mode,
  -- which would abort the user's INSERT/UPDATE on the triggered table when
  -- the volume has not yet been registered (no `fs_volume_state` row, e.g.
  -- before `snapshot.enable`). PG silently leaves `state` NULL and the
  -- following `IF NOT FOUND` would handle it, but OG never gets there.
  -- The `IF NOT EXISTS` short-circuit works on both backends.
  IF NOT EXISTS (SELECT 1 FROM fs_volume_state WHERE volume_id = vid) THEN
    RETURN NULL;
  END IF;
  SELECT rollback_enabled, current_snap_id INTO state
    FROM fs_volume_state WHERE volume_id = vid;
  IF NOT state.rollback_enabled THEN RETURN NULL; END IF;

  IF TG_OP = 'INSERT' THEN
    INSERT INTO fs_symlink_undo(volume_id, snap_id, op, ino)
      VALUES (vid, state.current_snap_id, 'I', NEW.ino);
  ELSIF TG_OP = 'UPDATE' THEN
    INSERT INTO fs_symlink_undo(volume_id, snap_id, op, ino, target)
      VALUES (vid, state.current_snap_id, 'U', OLD.ino, OLD.target);
  ELSIF TG_OP = 'DELETE' THEN
    INSERT INTO fs_symlink_undo(volume_id, snap_id, op, ino, target)
      VALUES (vid, state.current_snap_id, 'D', OLD.ino, OLD.target);
  END IF;
  RETURN NULL;
END $$ LANGUAGE plpgsql;
"#;

const BIND_FS_SYMLINK: &str = r#"
DROP TRIGGER IF EXISTS tr_fs_symlink_undo ON fs_symlink;
CREATE TRIGGER tr_fs_symlink_undo AFTER INSERT OR UPDATE OR DELETE ON fs_symlink
  FOR EACH ROW EXECUTE FUNCTION fs_symlink_capture_undo();
"#;

const FN_KV_STORE: &str = r#"
CREATE OR REPLACE FUNCTION kv_store_capture_undo() RETURNS trigger AS $$
DECLARE
  vid  text := current_setting('secafs.volume_id',     true);
  supp text := current_setting('secafs.suppress_undo', true);
  state record;
BEGIN
  IF vid IS NULL OR vid = '' THEN RETURN NULL; END IF;
  IF supp = 'true' THEN RETURN NULL; END IF;
  -- Guard against zero-row `SELECT INTO`: OpenGauss 6.0 raises
  -- `query returned no rows when process INTO` in its default plpgsql mode,
  -- which would abort the user's INSERT/UPDATE on the triggered table when
  -- the volume has not yet been registered (no `fs_volume_state` row, e.g.
  -- before `snapshot.enable`). PG silently leaves `state` NULL and the
  -- following `IF NOT FOUND` would handle it, but OG never gets there.
  -- The `IF NOT EXISTS` short-circuit works on both backends.
  IF NOT EXISTS (SELECT 1 FROM fs_volume_state WHERE volume_id = vid) THEN
    RETURN NULL;
  END IF;
  SELECT rollback_enabled, current_snap_id INTO state
    FROM fs_volume_state WHERE volume_id = vid;
  IF NOT state.rollback_enabled THEN RETURN NULL; END IF;

  IF TG_OP = 'INSERT' THEN
    INSERT INTO kv_store_undo(volume_id, snap_id, op, key)
      VALUES (vid, state.current_snap_id, 'I', NEW.key);
  ELSIF TG_OP = 'UPDATE' THEN
    INSERT INTO kv_store_undo(volume_id, snap_id, op, key, value, created_at, updated_at)
      VALUES (vid, state.current_snap_id, 'U', OLD.key, OLD.value, OLD.created_at, OLD.updated_at);
  ELSIF TG_OP = 'DELETE' THEN
    INSERT INTO kv_store_undo(volume_id, snap_id, op, key, value, created_at, updated_at)
      VALUES (vid, state.current_snap_id, 'D', OLD.key, OLD.value, OLD.created_at, OLD.updated_at);
  END IF;
  RETURN NULL;
END $$ LANGUAGE plpgsql;
"#;

const BIND_KV_STORE: &str = r#"
DROP TRIGGER IF EXISTS tr_kv_store_undo ON kv_store;
CREATE TRIGGER tr_kv_store_undo AFTER INSERT OR UPDATE OR DELETE ON kv_store
  FOR EACH ROW EXECUTE FUNCTION kv_store_capture_undo();
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ddl_statements_have_create_or_replace() {
        let stmts = ddl_statements();
        assert_eq!(stmts.len(), 10);
        let fns: Vec<_> = stmts.iter().filter(|s| s.contains("CREATE OR REPLACE FUNCTION")).collect();
        let triggers: Vec<_> = stmts.iter().filter(|s| s.contains("CREATE TRIGGER")).collect();
        assert_eq!(fns.len(), 5);
        assert_eq!(triggers.len(), 5);
    }
}
