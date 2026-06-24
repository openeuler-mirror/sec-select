"""Guard functions for filesystem operations validation"""

from typing import Any, Dict, Optional

from .db import DbConnection

from .constants import S_IFDIR, S_IFLNK, S_IFMT
from .errors import ErrnoException, FsSyscall


async def _get_inode_mode(db: DbConnection, ino: int) -> Optional[int]:
    """Return the inode's st_mode, or None if the inode does not exist."""
    cursor = await db.execute("SELECT mode FROM fs_inode WHERE ino = ?", (ino,))
    row = await cursor.fetchone()
    return row[0] if row else None


def _is_dir_mode(mode: int) -> bool:
    return (mode & S_IFMT) == S_IFDIR


async def get_inode_mode_or_throw(
    db: DbConnection,
    ino: int,
    syscall: FsSyscall,
    path: str,
) -> int:
    """Get inode mode or throw ENOENT if not found"""
    mode = await _get_inode_mode(db, ino)
    if mode is None:
        raise ErrnoException(
            code="ENOENT",
            syscall=syscall,
            path=path,
            message="no such file or directory",
        )
    return mode


def assert_not_root(path: str, syscall: FsSyscall) -> None:
    """Raise EPERM if path is the root directory (rm of root is the exception)."""
    if path == "/":
        if syscall == "rm":
            return  # rm("/", recursive=True) is allowed; caller must use clear_root or recursive
        raise ErrnoException(
            code="EPERM",
            syscall=syscall,
            path=path,
            message="operation not permitted on root directory",
        )


def normalize_rm_options(options: Optional[Dict[str, Any]]) -> Dict[str, bool]:
    """Coerce the optional rm options dict into {force, recursive} booleans."""
    return {
        "force": options.get("force", False) if options else False,
        "recursive": options.get("recursive", False) if options else False,
    }


def throw_enoent_unless_force(path: str, syscall: FsSyscall, force: bool) -> None:
    """Raise ENOENT for a missing target unless force is set (e.g. rm -f)."""
    if force:
        return
    raise ErrnoException(
        code="ENOENT",
        syscall=syscall,
        path=path,
        message="no such file or directory",
    )


def assert_not_symlink_mode(mode: int, syscall: FsSyscall, path: str) -> None:
    """Raise ENOSYS if mode is a symlink (symlinks are not yet supported)."""
    if (mode & S_IFMT) == S_IFLNK:
        raise ErrnoException(
            code="ENOSYS",
            syscall=syscall,
            path=path,
            message="symbolic links not supported yet",
        )


async def assert_existing_regular_inode(
    db: DbConnection,
    ino: int,
    syscall: FsSyscall,
    full_path_for_error: str,
) -> None:
    """Raise ENOENT if missing, EISDIR if a directory, ENOSYS if a symlink."""
    mode = await _get_inode_mode(db, ino)
    if mode is None:
        raise ErrnoException(
            code="ENOENT",
            syscall=syscall,
            path=full_path_for_error,
            message="no such file or directory",
        )
    if _is_dir_mode(mode):
        raise ErrnoException(
            code="EISDIR",
            syscall=syscall,
            path=full_path_for_error,
            message="illegal operation on a directory",
        )
    assert_not_symlink_mode(mode, syscall, full_path_for_error)


async def assert_inode_is_directory(
    db: DbConnection,
    ino: int,
    syscall: FsSyscall,
    full_path_for_error: str,
) -> None:
    """Raise ENOENT if missing, ENOTDIR if the inode is not a directory."""
    mode = await _get_inode_mode(db, ino)
    if mode is None:
        raise ErrnoException(
            code="ENOENT",
            syscall=syscall,
            path=full_path_for_error,
            message="no such file or directory",
        )
    if not _is_dir_mode(mode):
        raise ErrnoException(
            code="ENOTDIR",
            syscall=syscall,
            path=full_path_for_error,
            message="not a directory",
        )


async def assert_readdir_target_inode(
    db: DbConnection,
    ino: int,
    full_path_for_error: str,
) -> None:
    """Assert inode is a valid readdir target (directory, not symlink)"""
    syscall: FsSyscall = "scandir"
    mode = await _get_inode_mode(db, ino)
    if mode is None:
        raise ErrnoException(
            code="ENOENT",
            syscall=syscall,
            path=full_path_for_error,
            message="no such file or directory",
        )
    assert_not_symlink_mode(mode, syscall, full_path_for_error)
    if not _is_dir_mode(mode):
        raise ErrnoException(
            code="ENOTDIR",
            syscall=syscall,
            path=full_path_for_error,
            message="not a directory",
        )


async def assert_unlink_target_inode(
    db: DbConnection,
    ino: int,
    full_path_for_error: str,
) -> None:
    """Assert inode is a valid unlink target (file, not directory/symlink)"""
    syscall: FsSyscall = "unlink"
    mode = await _get_inode_mode(db, ino)
    if mode is None:
        raise ErrnoException(
            code="ENOENT",
            syscall=syscall,
            path=full_path_for_error,
            message="no such file or directory",
        )
    if _is_dir_mode(mode):
        raise ErrnoException(
            code="EISDIR",
            syscall=syscall,
            path=full_path_for_error,
            message="illegal operation on a directory",
        )
    assert_not_symlink_mode(mode, syscall, full_path_for_error)
