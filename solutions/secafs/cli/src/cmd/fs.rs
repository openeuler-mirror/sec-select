use std::collections::VecDeque;

use secafs_sdk::SecAFSOptions;
use anyhow::{Context, Result as AnyhowResult};
use secafs_sdk::db::DbValue as Value;

use crate::cmd::init::open_secafs;

const ROOT_INO: i64 = 1;
const S_IFMT: u32 = 0o170000;
const S_IFDIR: u32 = 0o040000;
const S_IFREG: u32 = 0o100000;
const S_IFLNK: u32 = 0o120000;

pub async fn ls_filesystem(
    stdout: &mut impl std::io::Write,
    postgres_url: String,
    path: &str,
) -> AnyhowResult<()> {
    let options = SecAFSOptions::resolve(&postgres_url)?;
    eprintln!("Using database: {}", postgres_url);

    let secafs_inst = open_secafs(options).await?;
    let conn = secafs_inst.get_connection().await?;

    if path != "/" {
        anyhow::bail!("Only root directory (/) is currently supported");
    }

    let mut queue: VecDeque<(i64, String)> = VecDeque::new();
    queue.push_back((ROOT_INO, String::new()));

    while let Some((parent_ino, prefix)) = queue.pop_front() {
        let query = format!(
            "SELECT d.name, d.ino, i.mode FROM fs_dentry d
             JOIN fs_inode i ON d.ino = i.ino
             WHERE d.parent_ino = {}
             ORDER BY d.name",
            parent_ino
        );

        let mut rows = conn
            .query(&query, ())
            .await
            .context("Failed to query directory entries")?;

        let mut entries = Vec::new();
        while let Some(row) = rows.next().await.context("Failed to fetch row")? {
            let name: String = row
                .get_value(0)
                .ok()
                .and_then(|v| {
                    if let Value::Text(s) = v {
                        Some(s.clone())
                    } else {
                        None
                    }
                })
                .unwrap_or_default();

            let ino: i64 = row
                .get_value(1)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0);

            let mode: u32 = row
                .get_value(2)
                .ok()
                .and_then(|v| v.as_integer().copied())
                .unwrap_or(0) as u32;

            entries.push((name, ino, mode));
        }

        for (name, ino, mode) in entries {
            let is_dir = mode & S_IFMT == S_IFDIR;
            let full_path = if prefix.is_empty() {
                name.clone()
            } else {
                format!("{}/{}", prefix, name)
            };

            stdout
                .write_fmt(format_args!(
                    "{} {}\n",
                    if is_dir { 'd' } else { 'f' },
                    full_path
                ))
                .context("Failed to write to stdout")?;

            if is_dir {
                queue.push_back((ino, full_path));
            }
        }
    }

    Ok(())
}

pub async fn cat_filesystem(
    stdout: &mut impl std::io::Write,
    postgres_url: String,
    path: &str,
) -> AnyhowResult<()> {
    let options = SecAFSOptions::resolve(&postgres_url)?;
    let secafs_inst = open_secafs(options).await?;

    match secafs_inst.fs.read_file(path).await? {
        Some(file) => {
            stdout.write_all(&file)?;
            Ok(())
        }
        None => anyhow::bail!("File not found: {}", path),
    }
}

pub async fn write_filesystem(
    postgres_url: String,
    path: &str,
    content: &str,
) -> AnyhowResult<()> {
    let options = SecAFSOptions::resolve(&postgres_url)?;
    let secafs_inst = open_secafs(options).await?;

    let mut components = path.split("/").collect::<Vec<_>>();
    if !path.starts_with("/") {
        components.insert(0, "");
    }
    for i in 2..components.len() {
        let dir_path = components[0..i].join("/");
        if secafs_inst.fs.stat(&dir_path).await?.is_none() {
            secafs_inst.fs.mkdir(&dir_path, 0, 0).await?;
        }
    }
    if secafs_inst.fs.stat(path).await?.is_some() {
        secafs_inst.fs.remove(path).await?;
    }
    let (_, file) = secafs_inst.fs.create_file(path, S_IFREG | 0o644, 0, 0).await?;
    file.pwrite(0, content.as_bytes()).await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ChangeType {
    Added,
    Modified,
    Deleted,
}

impl std::fmt::Display for ChangeType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ChangeType::Added => write!(f, "A"),
            ChangeType::Modified => write!(f, "M"),
            ChangeType::Deleted => write!(f, "D"),
        }
    }
}

fn file_type_char(mode: u32) -> char {
    match mode & S_IFMT {
        S_IFDIR => 'd',
        S_IFLNK => 'l',
        S_IFREG => 'f',
        _ => '?',
    }
}

fn path_exists_in_base(base_path: &str, rel_path: &str) -> bool {
    let full_path = format!("{}{}", base_path, rel_path);
    std::path::Path::new(&full_path).exists()
}

pub async fn diff_filesystem(postgres_url: String) -> AnyhowResult<()> {
    let options = SecAFSOptions::resolve(&postgres_url)?;
    eprintln!("Using database: {}", postgres_url);

    let secafs_inst = open_secafs(options).await?;

    let base_path = match secafs_inst.is_overlay_enabled().await? {
        Some(path) => path,
        None => {
            println!("No diff (non-overlay filesystem)");
            return Ok(());
        }
    };

    eprintln!("Base: {}", base_path);

    let mut changes: Vec<(ChangeType, char, String)> = Vec::new();

    let delta_paths = secafs_inst.get_delta_paths().await?;
    let whiteouts = secafs_inst.get_whiteouts().await?;

    for path in &delta_paths {
        let mode = secafs_inst.get_file_mode(path).await?.unwrap_or(0);
        let type_char = file_type_char(mode);

        if path_exists_in_base(&base_path, path) {
            changes.push((ChangeType::Modified, type_char, path.clone()));
        } else {
            changes.push((ChangeType::Added, type_char, path.clone()));
        }
    }

    for path in &whiteouts {
        let full_path = format!("{}{}", base_path, path);
        let base_path_obj = std::path::Path::new(&full_path);
        let type_char = if base_path_obj.is_dir() {
            'd'
        } else if base_path_obj.is_symlink() {
            'l'
        } else if base_path_obj.is_file() {
            'f'
        } else {
            '?'
        };

        changes.push((ChangeType::Deleted, type_char, path.clone()));
    }

    changes.sort_by(|a, b| a.2.cmp(&b.2));

    if changes.is_empty() {
        println!("No changes");
    } else {
        for (change_type, type_char, path) in changes {
            println!("{} {} {}", change_type, type_char, path);
        }
    }

    Ok(())
}
