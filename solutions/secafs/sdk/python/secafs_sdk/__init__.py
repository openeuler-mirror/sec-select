"""SecAFS Python SDK

A filesystem and key-value store for AI agents, powered by a database backend.
"""

from .secafs import SecAFS, SecAFSOptions, Task
from .errors import ErrnoException, FsErrorCode, FsSyscall
from .filesystem import S_IFDIR, S_IFLNK, S_IFMT, S_IFREG, Filesystem, Stats
from .kvstore import KvStore
from .toolcalls import ToolCall, ToolCalls, ToolCallStats

__version__ = "0.6.0"

__all__ = [
    "SecAFS",
    "SecAFSOptions",
    "Task",
    "KvStore",
    "Filesystem",
    "Stats",
    "S_IFDIR",
    "S_IFLNK",
    "S_IFMT",
    "S_IFREG",
    "ToolCalls",
    "ToolCall",
    "ToolCallStats",
    "ErrnoException",
    "FsErrorCode",
    "FsSyscall",
]
