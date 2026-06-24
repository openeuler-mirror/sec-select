use crate::db::{DbConn as Connection, DbValue as Value};
use crate::error::Result;
use async_trait::async_trait;
use std::{
    collections::{HashMap, HashSet},
    sync::{
        atomic::{AtomicI64, Ordering},
        Arc, RwLock,
    },
    time::{SystemTime, UNIX_EPOCH},
};
use tracing::trace;

use super::{
    secafs::SecAFS, BoxedFile, DirEntry, FileSystem, FilesystemStats, FsError, Stats, TimeChange,
};

/// Root inode number (matches FUSE convention)
const ROOT_INO: i64 = 1;

/// Which layer an inode belongs to
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Layer {
    Delta,
    Base,
}

/// Information about an inode in the overlay filesystem
#[derive(Debug, Clone)]
struct InodeInfo {
    /// Which layer this inode lives in
    layer: Layer,
    /// The inode number in the underlying layer
    underlying_ino: i64,
    /// Virtual path (for whiteout and copy-up operations)
    path: String,
}

/// A copy-on-write overlay filesystem using inode-based operations.
///
/// Combines a read-only base layer with a writable delta layer (SecAFS).
/// All modifications are written to the delta layer, while reads fall back
/// to the base layer if not found in delta.
pub struct OverlayFS {
    /// The underlying read-only base filesystem
    base: Arc<dyn FileSystem>,
    /// The delta layer where modifications go
    delta: SecAFS,
    /// Map from overlay inode to underlying layer info
    inode_map: RwLock<HashMap<i64, InodeInfo>>,
    /// Reverse map: (layer, underlying_ino) -> overlay_ino
    reverse_map: RwLock<HashMap<(Layer, i64), i64>>,
    /// Map from path to overlay inode (for path-based operations)
    path_map: RwLock<HashMap<String, i64>>,
    /// Next inode number to allocate
    next_ino: AtomicI64,
    /// Set of whiteout paths (deleted from base)
    whiteouts: RwLock<HashSet<String>>,
    /// Origin mapping: delta_ino -> base_ino (for copy-up consistency)
    origin_map: RwLock<HashMap<i64, i64>>,
}

impl OverlayFS {
    /// Create a new overlay filesystem
    pub fn new(base: Arc<dyn FileSystem>, delta: SecAFS) -> Self {
        let mut inode_map = HashMap::new();
        let mut reverse_map = HashMap::new();
        let mut path_map = HashMap::new();

        // Root inode maps to delta's root (inode 1)
        inode_map.insert(
            ROOT_INO,
            InodeInfo {
                layer: Layer::Delta,
                underlying_ino: 1,
                path: "/".to_string(),
            },
        );
        reverse_map.insert((Layer::Delta, 1), ROOT_INO);
        path_map.insert("/".to_string(), ROOT_INO);

        Self {
            base,
            delta,
            inode_map: RwLock::new(inode_map),
            reverse_map: RwLock::new(reverse_map),
            path_map: RwLock::new(path_map),
            next_ino: AtomicI64::new(2),
            whiteouts: RwLock::new(HashSet::new()),
            origin_map: RwLock::new(HashMap::new()),
        }
    }

    /// Initialize the overlay filesystem schema
    pub async fn init_schema(conn: &Connection, base_path: &str) -> Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_whiteout (
                path TEXT PRIMARY KEY,
                created_at BIGINT NOT NULL
            )",
            (),
        )
        .await?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_overlay_config (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            )",
            (),
        )
        .await?;
        conn.execute(
            "INSERT INTO fs_overlay_config (key, value) VALUES ('base_path', ?)
            ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
            (Value::Text(base_path.to_string()),),
        )
        .await?;
        conn.execute(
            "CREATE TABLE IF NOT EXISTS fs_origin (
                delta_ino BIGINT PRIMARY KEY,
                base_ino BIGINT NOT NULL
            )",
            (),
        )
        .await?;
        Ok(())
    }

    /// Initialize the overlay filesystem
    pub async fn init(&self, base_path: &str) -> Result<()> {
        let conn = self.delta.get_connection().await?;
        Self::init_schema(&conn, base_path).await?;
        self.load_whiteouts(&conn).await?;
        self.load_origins(&conn).await?;
        Ok(())
    }

    /// Load whiteouts from database into memory
    async fn load_whiteouts(&self, conn: &Connection) -> Result<()> {
        let mut rows = conn.query("SELECT path FROM fs_whiteout", ()).await?;
        let mut paths = Vec::new();
        while let Some(row) = rows.next().await? {
            if let Some(path) = row.get_value(0).ok().and_then(|v| match v {
                Value::Text(s) => Some(s.clone()),
                _ => None,
            }) {
                paths.push(path);
            }
        }
        let mut whiteouts = self.whiteouts.write().unwrap();
        for path in paths {
            whiteouts.insert(path);
        }
        Ok(())
    }

    /// Load existing whiteouts (public interface)
    pub async fn load_whiteouts_public(&self) -> Result<()> {
        let conn = self.delta.get_connection().await?;
        self.load_whiteouts(&conn).await
    }

    /// Load persisted state (whiteouts and origin mappings) from database.
    /// Call this after creating an OverlayFS for an existing database.
    pub async fn load(&self) -> Result<()> {
        let conn = self.delta.get_connection().await?;
        self.load_whiteouts(&conn).await?;
        self.load_origins(&conn).await?;
        Ok(())
    }

    /// Load origin mappings from database
    async fn load_origins(&self, conn: &Connection) -> Result<()> {
        let result = conn
            .query("SELECT delta_ino, base_ino FROM fs_origin", ())
            .await;
        if let Ok(mut rows) = result {
            let mut mappings = Vec::new();
            while let Some(row) = rows.next().await? {
                let delta_ino = row.get_value(0).ok().and_then(|v| v.as_integer().copied());
                let base_ino = row.get_value(1).ok().and_then(|v| v.as_integer().copied());
                if let (Some(d), Some(b)) = (delta_ino, base_ino) {
                    mappings.push((d, b));
                }
            }
            let mut origins = self.origin_map.write().unwrap();
            for (d, b) in mappings {
                origins.insert(d, b);
            }
        }
        Ok(())
    }

    /// Check if a path is whiteout (deleted from base)
    fn is_whiteout(&self, path: &str) -> bool {
        let whiteouts = self.whiteouts.read().unwrap();
        // Check path and all ancestors
        let mut current = String::new();
        for component in path.split('/').filter(|s| !s.is_empty()) {
            current = format!("{}/{}", current, component);
            if whiteouts.contains(&current) {
                return true;
            }
        }
        false
    }

    /// Create a whiteout for a path
    async fn create_whiteout(&self, path: &str) -> Result<()> {
        let conn = self.delta.get_connection().await?;
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs() as i64;
        let sql = "INSERT INTO fs_whiteout (path, created_at) VALUES (?, ?)
                ON CONFLICT (path) DO UPDATE SET created_at = EXCLUDED.created_at";
        conn.execute(sql, (path, now)).await?;
        self.whiteouts.write().unwrap().insert(path.to_string());
        Ok(())
    }

    /// Remove a whiteout
    async fn remove_whiteout(&self, path: &str) -> Result<()> {
        if !self.whiteouts.read().unwrap().contains(path) {
            return Ok(());
        }
        let conn = self.delta.get_connection().await?;
        conn.execute("DELETE FROM fs_whiteout WHERE path = ?", (path,))
            .await?;
        self.whiteouts.write().unwrap().remove(path);
        Ok(())
    }

    /// Get child whiteouts for a directory
    fn get_child_whiteouts(&self, dir_path: &str) -> HashSet<String> {
        let whiteouts = self.whiteouts.read().unwrap();
        let prefix = if dir_path == "/" {
            "/".to_string()
        } else {
            format!("{}/", dir_path)
        };
        whiteouts
            .iter()
            .filter_map(|p| {
                if dir_path == "/" {
                    // Direct children of root
                    let trimmed = p.trim_start_matches('/');
                    if !trimmed.contains('/') {
                        Some(trimmed.to_string())
                    } else {
                        None
                    }
                } else if p.starts_with(&prefix) {
                    let rest = &p[prefix.len()..];
                    if !rest.contains('/') {
                        Some(rest.to_string())
                    } else {
                        None
                    }
                } else {
                    None
                }
            })
            .collect()
    }

    /// Allocate a new overlay inode number
    fn alloc_ino(&self) -> i64 {
        self.next_ino.fetch_add(1, Ordering::Relaxed)
    }

    /// Get or create an overlay inode for a layer inode
    fn get_or_create_overlay_ino(&self, layer: Layer, underlying_ino: i64, path: &str) -> i64 {
        // Check reverse map first
        {
            let reverse = self.reverse_map.read().unwrap();
            if let Some(&ino) = reverse.get(&(layer, underlying_ino)) {
                return ino;
            }
        }

        // Allocate new inode
        let ino = self.alloc_ino();
        {
            let mut inode_map = self.inode_map.write().unwrap();
            inode_map.insert(
                ino,
                InodeInfo {
                    layer,
                    underlying_ino,
                    path: path.to_string(),
                },
            );
        }
        {
            let mut reverse = self.reverse_map.write().unwrap();
            reverse.insert((layer, underlying_ino), ino);
        }
        {
            let mut path_map = self.path_map.write().unwrap();
            path_map.insert(path.to_string(), ino);
        }

        ino
    }

    /// Get inode info for an overlay inode
    fn get_inode_info(&self, ino: i64) -> Option<InodeInfo> {
        self.inode_map.read().unwrap().get(&ino).cloned()
    }

    /// Build path from parent inode and name
    fn build_path(&self, parent_ino: i64, name: &str) -> Result<String> {
        let info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        Ok(if info.path == "/" {
            format!("/{}", name)
        } else {
            format!("{}/{}", info.path, name)
        })
    }

    /// Get a reference to the base layer
    pub fn base(&self) -> &Arc<dyn FileSystem> {
        &self.base
    }

    /// Get a reference to the delta layer
    pub fn delta(&self) -> &SecAFS {
        &self.delta
    }

    /// Store origin mapping for copy-up
    async fn add_origin_mapping(&self, delta_ino: i64, base_ino: i64) -> Result<()> {
        let conn = self.delta.get_connection().await?;
        let sql = "INSERT INTO fs_origin (delta_ino, base_ino) VALUES (?, ?)
                ON CONFLICT (delta_ino) DO UPDATE SET base_ino = EXCLUDED.base_ino";
        conn.execute(sql, (delta_ino, base_ino)).await?;
        self.origin_map.write().unwrap().insert(delta_ino, base_ino);
        Ok(())
    }

    /// Get origin inode for a delta inode
    fn get_origin_ino(&self, delta_ino: i64) -> Option<i64> {
        self.origin_map.read().unwrap().get(&delta_ino).copied()
    }

    /// Promote an overlay inode from base layer to delta layer.
    ///
    /// When a directory that was originally looked up from base gets a
    /// corresponding directory created in delta (via ensure_parent_dirs),
    /// we need to update the overlay inode to point to delta. This ensures
    /// that operations like readdir and unlink will check the delta layer.
    fn promote_to_delta(&self, path: &str, delta_ino: i64) {
        let path_map = self.path_map.read().unwrap();
        let overlay_ino = match path_map.get(path) {
            Some(&ino) => ino,
            None => return, // No existing mapping, nothing to promote
        };
        drop(path_map);

        // Update the inode mapping to point to delta
        let mut inode_map = self.inode_map.write().unwrap();
        if let Some(info) = inode_map.get_mut(&overlay_ino) {
            if info.layer == Layer::Base {
                let old_base_ino = info.underlying_ino;
                info.layer = Layer::Delta;
                info.underlying_ino = delta_ino;

                // Update reverse map: add delta mapping (keep base mapping for origin lookups)
                drop(inode_map);
                let mut reverse = self.reverse_map.write().unwrap();
                reverse.remove(&(Layer::Base, old_base_ino));
                reverse.insert((Layer::Delta, delta_ino), overlay_ino);
            }
        }
    }

    /// Ensure parent directories exist in delta layer
    async fn ensure_parent_dirs(&self, path: &str, uid: u32, gid: u32) -> Result<()> {
        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();

        let mut current_path = String::new();
        let mut current_delta_ino: i64 = 1; // Delta root
        let mut current_base_ino: i64 = 1; // Base root

        for component in components.iter().take(components.len().saturating_sub(1)) {
            current_path = format!("{}/{}", current_path, component);

            // Remove any whiteout for this path
            self.remove_whiteout(&current_path).await?;

            // Check if directory exists in delta
            if let Some(stats) =
                FileSystem::lookup(&self.delta, current_delta_ino, component).await?
            {
                if stats.is_directory() {
                    current_delta_ino = stats.ino;
                    // Advance base in parallel so it stays in sync
                    if let Some(bs) = self.base.lookup(current_base_ino, component).await? {
                        current_base_ino = bs.ino;
                    }
                    continue;
                } else {
                    return Err(FsError::NotADirectory.into());
                }
            }

            // Not in delta, check base (using the base inode, not delta inode)
            let base_stats = self.base.lookup(current_base_ino, component).await?;
            let (dir_uid, dir_gid, origin_base_ino) = if let Some(s) = &base_stats {
                let base_ino = s.ino;
                current_base_ino = base_ino;
                (s.uid, s.gid, Some(base_ino))
            } else {
                (uid, gid, None)
            };

            // Create directory in delta
            let new_stats = FileSystem::mkdir(
                &self.delta,
                current_delta_ino,
                component,
                0o755,
                dir_uid,
                dir_gid,
            )
            .await?;
            current_delta_ino = new_stats.ino;

            // Create origin mapping if directory exists in base, so that
            // lookups return consistent overlay inodes
            if let Some(base_ino) = origin_base_ino {
                self.add_origin_mapping(new_stats.ino, base_ino).await?;
                // Promote the overlay inode to delta so readdir/unlink will check delta
                self.promote_to_delta(&current_path, new_stats.ino);
            }
        }

        Ok(())
    }

    /// Copy a file from base to delta for modification
    async fn copy_up(&self, path: &str, base_ino: i64) -> Result<i64> {
        // Parse path to get parent and name
        let components: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
        if components.is_empty() {
            return Err(FsError::RootOperation.into());
        }
        let name = components.last().unwrap();

        // Check if already copied up - walk delta to find parent and check for file
        let mut parent_ino: i64 = 1;
        let mut found_parent = true;
        for comp in components.iter().take(components.len() - 1) {
            if let Some(stats) = FileSystem::lookup(&self.delta, parent_ino, comp).await? {
                parent_ino = stats.ino;
            } else {
                found_parent = false;
                break;
            }
        }

        // If parent exists in delta, check if file already exists there
        if found_parent {
            if let Some(stats) = FileSystem::lookup(&self.delta, parent_ino, name).await? {
                // Already copied up, return delta inode
                return Ok(stats.ino);
            }
        }

        // Get base stats
        let base_stats = self
            .base
            .getattr(base_ino)
            .await?
            .ok_or(FsError::NotFound)?;

        // Ensure parent directories exist
        self.ensure_parent_dirs(path, base_stats.uid, base_stats.gid)
            .await?;

        // Look up parent in delta by walking the path
        let mut parent_ino: i64 = 1; // Start at delta root
        for comp in components.iter().take(components.len() - 1) {
            let stats = FileSystem::lookup(&self.delta, parent_ino, comp)
                .await?
                .ok_or(FsError::NotFound)?;
            parent_ino = stats.ino;
        }

        // Copy based on file type
        let delta_ino = if base_stats.is_symlink() {
            let target = self
                .base
                .readlink(base_ino)
                .await?
                .ok_or(FsError::NotFound)?;
            let stats = FileSystem::symlink(
                &self.delta,
                parent_ino,
                name,
                &target,
                base_stats.uid,
                base_stats.gid,
            )
            .await?;
            stats.ino
        } else if base_stats.is_directory() {
            let stats = FileSystem::mkdir(
                &self.delta,
                parent_ino,
                name,
                base_stats.mode & 0o7777,
                base_stats.uid,
                base_stats.gid,
            )
            .await?;
            stats.ino
        } else {
            // Regular file - read content and create
            let base_file = self.base.open(base_ino, libc::O_RDONLY).await?;
            let content = base_file.pread(0, base_stats.size as u64).await?;

            let (stats, delta_file) = FileSystem::create_file(
                &self.delta,
                parent_ino,
                name,
                base_stats.mode,
                base_stats.uid,
                base_stats.gid,
            )
            .await?;
            delta_file.pwrite(0, &content).await?;
            stats.ino
        };

        // Store origin mapping
        self.add_origin_mapping(delta_ino, base_ino).await?;

        Ok(delta_ino)
    }

    /// Copy-up a file and update the inode mapping so subsequent operations
    /// go to the delta layer. Returns the delta inode.
    async fn copy_up_and_update_mapping(&self, overlay_ino: i64, info: &InodeInfo) -> Result<i64> {
        let delta_ino = self.copy_up(&info.path, info.underlying_ino).await?;

        // Update the inode mapping to point to delta
        {
            let mut inode_map = self.inode_map.write().unwrap();
            inode_map.insert(
                overlay_ino,
                InodeInfo {
                    layer: Layer::Delta,
                    underlying_ino: delta_ino,
                    path: info.path.clone(),
                },
            );
        }
        {
            let mut reverse_map = self.reverse_map.write().unwrap();
            // Keep the base mapping so lookups via origin still return the same overlay inode
            // (Layer::Base, base_ino) -> overlay_ino is kept
            // Add the delta mapping as well
            reverse_map.insert((Layer::Delta, delta_ino), overlay_ino);
        }

        Ok(delta_ino)
    }
}

#[async_trait]
impl FileSystem for OverlayFS {
    async fn lookup(&self, parent_ino: i64, name: &str) -> Result<Option<Stats>> {
        trace!(
            "OverlayFS::lookup: parent_ino={}, name={}",
            parent_ino,
            name
        );

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        // Check for whiteout
        if self.is_whiteout(&path) {
            return Ok(None);
        }

        // Try delta first - need to find the corresponding delta parent
        let delta_parent_ino = if parent_info.layer == Layer::Delta {
            Some(parent_info.underlying_ino)
        } else {
            // Parent is in base, walk the path in delta to find corresponding directory
            let mut ino: i64 = 1; // Start at delta root
            let mut found_all = true;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = self.delta.lookup(ino, comp).await? {
                    ino = s.ino;
                } else {
                    found_all = false;
                    break;
                }
            }
            if found_all {
                Some(ino)
            } else {
                None
            }
        };

        // Look up in delta (only if we resolved the correct parent)
        if let Some(delta_stats) = match delta_parent_ino {
            Some(ino) => self.delta.lookup(ino, name).await?,
            None => None,
        } {
            let ino = self.get_or_create_overlay_ino(Layer::Delta, delta_stats.ino, &path);
            let mut stats = delta_stats;

            // Check for origin mapping to return stable inode
            if let Some(base_ino) = self.get_origin_ino(stats.ino) {
                stats.ino = self.get_or_create_overlay_ino(Layer::Base, base_ino, &path);
            } else {
                stats.ino = ino;
            }

            return Ok(Some(stats));
        }

        // Try base
        let base_parent_ino = if parent_info.layer == Layer::Base {
            parent_info.underlying_ino
        } else {
            // Need to find corresponding base parent by path
            // For root, use base root (1)
            if parent_info.path == "/" {
                1
            } else {
                // Walk the base to find the parent
                let mut base_ino: i64 = 1;
                for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                    if let Some(s) = self.base.lookup(base_ino, comp).await? {
                        base_ino = s.ino;
                    } else {
                        return Ok(None);
                    }
                }
                base_ino
            }
        };

        if let Some(base_stats) = self.base.lookup(base_parent_ino, name).await? {
            let ino = self.get_or_create_overlay_ino(Layer::Base, base_stats.ino, &path);
            let mut stats = base_stats;
            stats.ino = ino;
            return Ok(Some(stats));
        }

        Ok(None)
    }

    async fn getattr(&self, ino: i64) -> Result<Option<Stats>> {
        trace!("OverlayFS::getattr: ino={}", ino);

        let info = match self.get_inode_info(ino) {
            Some(i) => i,
            None => return Ok(None),
        };

        let stats = match info.layer {
            Layer::Delta => FileSystem::getattr(&self.delta, info.underlying_ino).await?,
            Layer::Base => self.base.getattr(info.underlying_ino).await?,
        };

        Ok(stats.map(|mut s| {
            s.ino = ino;
            s
        }))
    }

    async fn readlink(&self, ino: i64) -> Result<Option<String>> {
        trace!("OverlayFS::readlink: ino={}", ino);

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;

        match info.layer {
            Layer::Delta => FileSystem::readlink(&self.delta, info.underlying_ino).await,
            Layer::Base => self.base.readlink(info.underlying_ino).await,
        }
    }

    async fn readdir(&self, ino: i64) -> Result<Option<Vec<String>>> {
        trace!("OverlayFS::readdir: ino={}", ino);

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;
        let child_whiteouts = self.get_child_whiteouts(&info.path);

        let mut entries = HashSet::new();

        // Get delta entries
        if info.layer == Layer::Delta {
            if let Some(delta_entries) = self.delta.readdir(info.underlying_ino).await? {
                entries.extend(delta_entries);
            }
        }

        // Get base entries (need to resolve base inode from path)
        let base_ino = if info.layer == Layer::Base {
            Some(info.underlying_ino)
        } else {
            // Walk base to find corresponding directory
            let components: Vec<&str> = info.path.split('/').filter(|s| !s.is_empty()).collect();
            let mut ino: i64 = 1;
            let mut found_all = true;
            for comp in &components {
                if let Some(s) = self.base.lookup(ino, comp).await? {
                    ino = s.ino;
                } else {
                    found_all = false;
                    break;
                }
            }
            if found_all {
                Some(ino)
            } else {
                None
            }
        };

        if let Some(base_ino) = base_ino {
            if let Some(base_entries) = self.base.readdir(base_ino).await? {
                for entry in base_entries {
                    let entry_path = if info.path == "/" {
                        format!("/{}", entry)
                    } else {
                        format!("{}/{}", info.path, entry)
                    };
                    if !self.is_whiteout(&entry_path) && !child_whiteouts.contains(&entry) {
                        entries.insert(entry);
                    }
                }
            }
        }

        let mut result: Vec<_> = entries.into_iter().collect();
        result.sort();
        Ok(Some(result))
    }

    async fn readdir_plus(&self, ino: i64) -> Result<Option<Vec<DirEntry>>> {
        trace!("OverlayFS::readdir_plus: ino={}", ino);

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;
        let child_whiteouts = self.get_child_whiteouts(&info.path);

        let mut entries_map: HashMap<String, DirEntry> = HashMap::new();

        // Get base entries first (so delta can override)
        let base_ino = if info.layer == Layer::Base {
            Some(info.underlying_ino)
        } else {
            let components: Vec<&str> = info.path.split('/').filter(|s| !s.is_empty()).collect();
            let mut ino: i64 = 1;
            let mut found_all = true;
            for comp in &components {
                if let Some(s) = self.base.lookup(ino, comp).await? {
                    ino = s.ino;
                } else {
                    found_all = false;
                    break;
                }
            }
            if found_all {
                Some(ino)
            } else {
                None
            }
        };

        if let Some(base_ino) = base_ino {
            if let Some(base_entries) = self.base.readdir_plus(base_ino).await? {
                for mut entry in base_entries {
                    let entry_path = if info.path == "/" {
                        format!("/{}", entry.name)
                    } else {
                        format!("{}/{}", info.path, entry.name)
                    };

                    if !self.is_whiteout(&entry_path) && !child_whiteouts.contains(&entry.name) {
                        let overlay_ino = self.get_or_create_overlay_ino(
                            Layer::Base,
                            entry.stats.ino,
                            &entry_path,
                        );
                        entry.stats.ino = overlay_ino;
                        entries_map.insert(entry.name.clone(), entry);
                    }
                }
            }
        }

        // Get delta entries (override base)
        if info.layer == Layer::Delta {
            if let Some(delta_entries) = self.delta.readdir_plus(info.underlying_ino).await? {
                for mut entry in delta_entries {
                    let entry_path = if info.path == "/" {
                        format!("/{}", entry.name)
                    } else {
                        format!("{}/{}", info.path, entry.name)
                    };

                    // Check for origin mapping
                    if let Some(base_ino) = self.get_origin_ino(entry.stats.ino) {
                        entry.stats.ino =
                            self.get_or_create_overlay_ino(Layer::Base, base_ino, &entry_path);
                    } else {
                        let overlay_ino = self.get_or_create_overlay_ino(
                            Layer::Delta,
                            entry.stats.ino,
                            &entry_path,
                        );
                        entry.stats.ino = overlay_ino;
                    }

                    entries_map.insert(entry.name.clone(), entry);
                }
            }
        }

        let mut result: Vec<_> = entries_map.into_values().collect();
        result.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(Some(result))
    }

    async fn chmod(&self, ino: i64, mode: u32) -> Result<()> {
        trace!("OverlayFS::chmod: ino={}, mode={:o}", ino, mode);

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;

        let delta_ino = match info.layer {
            Layer::Delta => info.underlying_ino,
            Layer::Base => self.copy_up_and_update_mapping(ino, &info).await?,
        };

        self.delta.chmod(delta_ino, mode).await
    }

    async fn chown(&self, ino: i64, uid: Option<u32>, gid: Option<u32>) -> Result<()> {
        trace!(
            "OverlayFS::chown: ino={}, uid={:?}, gid={:?}",
            ino,
            uid,
            gid
        );

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;

        let delta_ino = match info.layer {
            Layer::Delta => info.underlying_ino,
            Layer::Base => self.copy_up_and_update_mapping(ino, &info).await?,
        };

        self.delta.chown(delta_ino, uid, gid).await
    }

    async fn utimens(&self, ino: i64, atime: TimeChange, mtime: TimeChange) -> Result<()> {
        trace!("OverlayFS::utimens: ino={}", ino);

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;

        let delta_ino = match info.layer {
            Layer::Delta => info.underlying_ino,
            Layer::Base => self.copy_up_and_update_mapping(ino, &info).await?,
        };

        self.delta.utimens(delta_ino, atime, mtime).await
    }

    async fn open(&self, ino: i64, flags: i32) -> Result<BoxedFile> {
        trace!("OverlayFS::open: ino={}", ino);

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;

        let delta_ino = match info.layer {
            Layer::Delta => info.underlying_ino,
            Layer::Base => self.copy_up_and_update_mapping(ino, &info).await?,
        };

        FileSystem::open(&self.delta, delta_ino, flags).await
    }

    async fn mkdir(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        trace!("OverlayFS::mkdir: parent_ino={}, name={}", parent_ino, name);

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        // Check if already exists
        if self.lookup(parent_ino, name).await?.is_some() {
            return Err(FsError::AlreadyExists.into());
        }

        // Remove whiteout if exists
        self.remove_whiteout(&path).await?;

        // Ensure parent dirs exist in delta
        self.ensure_parent_dirs(&path, uid, gid).await?;

        // Get delta parent inode
        let delta_parent_ino = if parent_info.layer == Layer::Delta {
            parent_info.underlying_ino
        } else {
            // Walk delta to find parent
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        let mut stats =
            FileSystem::mkdir(&self.delta, delta_parent_ino, name, mode, uid, gid).await?;
        let overlay_ino = self.get_or_create_overlay_ino(Layer::Delta, stats.ino, &path);
        stats.ino = overlay_ino;

        Ok(stats)
    }

    async fn create_file(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        uid: u32,
        gid: u32,
    ) -> Result<(Stats, BoxedFile)> {
        trace!(
            "OverlayFS::create_file: parent_ino={}, name={}",
            parent_ino,
            name
        );

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        // Remove whiteout if exists
        self.remove_whiteout(&path).await?;

        // Ensure parent dirs exist in delta
        self.ensure_parent_dirs(&path, uid, gid).await?;

        // Get delta parent inode
        let delta_parent_ino = if parent_info.layer == Layer::Delta {
            parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        let (mut stats, file) =
            FileSystem::create_file(&self.delta, delta_parent_ino, name, mode, uid, gid).await?;
        let overlay_ino = self.get_or_create_overlay_ino(Layer::Delta, stats.ino, &path);
        stats.ino = overlay_ino;

        Ok((stats, file))
    }

    async fn mknod(
        &self,
        parent_ino: i64,
        name: &str,
        mode: u32,
        rdev: u64,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        trace!("OverlayFS::mknod: parent_ino={}, name={}", parent_ino, name);

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        self.remove_whiteout(&path).await?;
        self.ensure_parent_dirs(&path, uid, gid).await?;

        let delta_parent_ino = if parent_info.layer == Layer::Delta {
            parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        let mut stats =
            FileSystem::mknod(&self.delta, delta_parent_ino, name, mode, rdev, uid, gid).await?;
        let overlay_ino = self.get_or_create_overlay_ino(Layer::Delta, stats.ino, &path);
        stats.ino = overlay_ino;

        Ok(stats)
    }

    async fn symlink(
        &self,
        parent_ino: i64,
        name: &str,
        target: &str,
        uid: u32,
        gid: u32,
    ) -> Result<Stats> {
        trace!(
            "OverlayFS::symlink: parent_ino={}, name={}, target={}",
            parent_ino,
            name,
            target
        );

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        self.remove_whiteout(&path).await?;
        self.ensure_parent_dirs(&path, uid, gid).await?;

        let delta_parent_ino = if parent_info.layer == Layer::Delta {
            parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        let mut stats =
            FileSystem::symlink(&self.delta, delta_parent_ino, name, target, uid, gid).await?;
        let overlay_ino = self.get_or_create_overlay_ino(Layer::Delta, stats.ino, &path);
        stats.ino = overlay_ino;

        Ok(stats)
    }

    async fn unlink(&self, parent_ino: i64, name: &str) -> Result<()> {
        trace!(
            "OverlayFS::unlink: parent_ino={}, name={}",
            parent_ino,
            name
        );

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        // Check if it exists
        let stats = self
            .lookup(parent_ino, name)
            .await?
            .ok_or(FsError::NotFound)?;
        if stats.is_directory() {
            return Err(FsError::IsADirectory.into());
        }

        // Try to remove from delta
        if parent_info.layer == Layer::Delta {
            let _ = FileSystem::unlink(&self.delta, parent_info.underlying_ino, name).await;
        }

        // Check if exists in base
        let base_parent_ino = if parent_info.layer == Layer::Base {
            parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = self.base.lookup(ino, comp).await? {
                    ino = s.ino;
                } else {
                    return Ok(()); // Parent doesn't exist in base
                }
            }
            ino
        };

        if self.base.lookup(base_parent_ino, name).await?.is_some() {
            self.create_whiteout(&path).await?;
        }

        Ok(())
    }

    async fn rmdir(&self, parent_ino: i64, name: &str) -> Result<()> {
        trace!("OverlayFS::rmdir: parent_ino={}, name={}", parent_ino, name);

        let parent_info = self.get_inode_info(parent_ino).ok_or(FsError::NotFound)?;
        let path = self.build_path(parent_ino, name)?;

        // Check if it exists and is a directory
        let stats = self
            .lookup(parent_ino, name)
            .await?
            .ok_or(FsError::NotFound)?;
        if !stats.is_directory() {
            return Err(FsError::NotADirectory.into());
        }

        // Check if directory is empty (in overlay view)
        let dir_entries = self.readdir(stats.ino).await?.unwrap_or_default();
        if !dir_entries.is_empty() {
            return Err(FsError::NotEmpty.into());
        }

        // Try to remove from delta
        if parent_info.layer == Layer::Delta {
            let _ = FileSystem::rmdir(&self.delta, parent_info.underlying_ino, name).await;
        }

        // Check if exists in base
        let base_parent_ino = if parent_info.layer == Layer::Base {
            parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = self.base.lookup(ino, comp).await? {
                    ino = s.ino;
                } else {
                    return Ok(());
                }
            }
            ino
        };

        if self.base.lookup(base_parent_ino, name).await?.is_some() {
            self.create_whiteout(&path).await?;
        }

        Ok(())
    }

    async fn link(&self, ino: i64, newparent_ino: i64, newname: &str) -> Result<Stats> {
        trace!(
            "OverlayFS::link: ino={}, newparent_ino={}, newname={}",
            ino,
            newparent_ino,
            newname
        );

        let info = self.get_inode_info(ino).ok_or(FsError::NotFound)?;
        let parent_info = self
            .get_inode_info(newparent_ino)
            .ok_or(FsError::NotFound)?;
        let new_path = self.build_path(newparent_ino, newname)?;

        // Ensure file is in delta (copy up if needed)
        let delta_ino = if info.layer == Layer::Delta {
            info.underlying_ino
        } else {
            self.copy_up(&info.path, info.underlying_ino).await?
        };

        self.remove_whiteout(&new_path).await?;
        self.ensure_parent_dirs(&new_path, 0, 0).await?;

        // Get delta parent
        let delta_parent_ino = if parent_info.layer == Layer::Delta {
            parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        let mut stats = FileSystem::link(&self.delta, delta_ino, delta_parent_ino, newname).await?;
        stats.ino = ino; // Keep original overlay inode

        Ok(stats)
    }

    async fn rename(
        &self,
        oldparent_ino: i64,
        oldname: &str,
        newparent_ino: i64,
        newname: &str,
    ) -> Result<()> {
        trace!(
            "OverlayFS::rename: oldparent={}, oldname={}, newparent={}, newname={}",
            oldparent_ino,
            oldname,
            newparent_ino,
            newname
        );

        let old_parent_info = self
            .get_inode_info(oldparent_ino)
            .ok_or(FsError::NotFound)?;
        let new_parent_info = self
            .get_inode_info(newparent_ino)
            .ok_or(FsError::NotFound)?;
        let old_path = self.build_path(oldparent_ino, oldname)?;
        let new_path = self.build_path(newparent_ino, newname)?;

        // Get source stats
        let src_stats = self
            .lookup(oldparent_ino, oldname)
            .await?
            .ok_or(FsError::NotFound)?;
        let src_info = self
            .get_inode_info(src_stats.ino)
            .ok_or(FsError::NotFound)?;

        // Ensure source is in delta
        let delta_src_parent_ino = if old_parent_info.layer == Layer::Delta {
            old_parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in old_parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        // If source is in base, copy to delta first
        if src_info.layer == Layer::Base {
            self.copy_up(&old_path, src_info.underlying_ino).await?;
        }

        // Remove whiteout at destination
        self.remove_whiteout(&new_path).await?;
        self.ensure_parent_dirs(&new_path, 0, 0).await?;

        // Get delta destination parent
        let delta_dst_parent_ino = if new_parent_info.layer == Layer::Delta {
            new_parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in new_parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = FileSystem::lookup(&self.delta, ino, comp).await? {
                    ino = s.ino;
                }
            }
            ino
        };

        // Perform rename in delta
        FileSystem::rename(
            &self.delta,
            delta_src_parent_ino,
            oldname,
            delta_dst_parent_ino,
            newname,
        )
        .await?;

        // Create whiteout at source if it existed in base
        let base_src_parent_ino = if old_parent_info.layer == Layer::Base {
            old_parent_info.underlying_ino
        } else {
            let mut ino: i64 = 1;
            for comp in old_parent_info.path.split('/').filter(|s| !s.is_empty()) {
                if let Some(s) = self.base.lookup(ino, comp).await? {
                    ino = s.ino;
                } else {
                    return Ok(());
                }
            }
            ino
        };

        if self
            .base
            .lookup(base_src_parent_ino, oldname)
            .await?
            .is_some()
        {
            self.create_whiteout(&old_path).await?;
        }

        Ok(())
    }

    async fn statfs(&self) -> Result<FilesystemStats> {
        FileSystem::statfs(&self.delta).await
    }

    async fn forget(&self, ino: i64, nlookup: u64) {
        // Look up the inode info to determine which layer it belongs to
        let info = match self.get_inode_info(ino) {
            Some(i) => i,
            None => return, // Unknown inode, nothing to forget
        };

        // Pass through to the appropriate layer
        match info.layer {
            Layer::Delta => {
                // Delta (SecAFS) doesn't cache fds, but call it anyway for completeness
                FileSystem::forget(&self.delta, info.underlying_ino, nlookup).await;
            }
            Layer::Base => {
                // Base layer (HostFS) caches O_PATH fds and needs forget
                self.base.forget(info.underlying_ino, nlookup).await;
            }
        }

        // Note: We don't remove from inode_map here because the overlay layer's
        // inode mapping is relatively lightweight (no fd). The base layer's
        // forget handles the actual fd cleanup.
    }
}

// Tests removed: they required SQLite (SecAFS::new) which is no longer supported.
// Overlay tests need a running PostgreSQL instance to function.
#[cfg(all(test, any(target_os = "linux", target_os = "macos")))]
mod tests {
    #[allow(unused_imports)]
    use super::*;
}
