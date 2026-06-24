"""Main SecAFS class"""

import json
from typing import List, Optional

from .db import DbConnection, connect_postgres

from .filesystem import Filesystem
from .kvstore import KvStore
from .toolcalls import ToolCalls


class SecAFSOptions:
    """Configuration options for opening a SecAFS instance

    Attributes:
        postgres_url: Postgres or OpenGauss connection URL (required).
            URLs with ``opengauss://`` scheme are automatically normalized
            to ``postgres://`` for driver compatibility.
        skip_schema_init: If True, do not run CREATE/ALTER (for limited DB roles that only have SELECT/INSERT/UPDATE/DELETE on existing tables)
        backend: Database backend type — ``"postgres"`` (default) or ``"opengauss"``.
            Auto-detected from the URL scheme when not explicitly set.
    """

    def __init__(self, postgres_url: str, skip_schema_init: bool = False, backend: Optional[str] = None):
        self.postgres_url = postgres_url
        self.skip_schema_init = skip_schema_init
        if backend is not None:
            self.backend = backend
        elif postgres_url.startswith("opengauss://"):
            self.backend = "opengauss"
        else:
            self.backend = "postgres"


class Task:
    """A transactional task that groups multiple filesystem operations.

    All operations performed through ``task.fs``, ``task.kv``, and
    ``task.tools`` share a single long-lived database transaction.
    Intermediate states can be captured with ``savepoint()`` and restored
    with ``rollback_to()``.  The whole task is atomically applied via
    ``commit()`` or discarded via ``abort()``.  On commit, SecAFS writes
    audit log entries for all filesystem mutations in the task (so audit
    is produced by SecAFS, not by the caller).
    """

    def __init__(self, db: DbConnection, connect_url: Optional[str] = None) -> None:
        self._db = db
        self._connect_url = connect_url
        self._closed = False
        self._savepoints: List[str] = []
        self._audit_trail: List[dict] = []

    async def _init_components(self) -> None:
        """Initialise fs / kv / tools on the task connection."""
        self.fs = await Filesystem.from_database(self._db, audit_trail=self._audit_trail)
        self.kv = await KvStore.from_database(self._db)
        self.tools = await ToolCalls.from_database(self._db)

    # -- savepoint management ------------------------------------------------

    async def savepoint(self, name: str) -> None:
        await self._db.savepoint(name)
        self._savepoints.append(name)

    async def rollback_to(self, name: str) -> None:
        if name not in self._savepoints:
            raise ValueError(f"Unknown savepoint: {name}")
        await self._db.rollback_to_savepoint(name)
        idx = self._savepoints.index(name)
        self._savepoints = self._savepoints[: idx + 1]

    @property
    def savepoints(self) -> List[str]:
        return list(self._savepoints)

    # -- finalisation --------------------------------------------------------

    async def commit(self) -> None:
        """Commit all changes and close the task connection.
        After committing the transaction, flushes the task's filesystem audit
        trail into tool_calls (in a separate connection) so audit is visible.
        """
        if self._closed:
            return
        self._closed = True
        await self._db.commit_transaction()
        await self._flush_audit_trail()
        await self._db.close()

    async def _flush_audit_trail(self) -> None:
        """Write task's filesystem audit trail to tool_calls in a new connection (committed)."""
        if not self._connect_url or not self._audit_trail:
            return
        from .db import connect_postgres
        db = await connect_postgres(self._connect_url)
        try:
            cur = await db.execute("SELECT current_user", ())
            row = await cur.fetchone()
            actor = row[0] if row else "unknown"
        except Exception:
            actor = "unknown"
        for entry in self._audit_trail:
            name = entry.get("name", "")
            path = entry.get("path", "")
            result_summary = entry.get("result_summary")
            error_msg = entry.get("error_msg")
            started_at = entry.get("started_at", 0)
            completed_at = entry.get("completed_at", 0)
            status = "error" if error_msg else "success"
            duration_ms = (completed_at - started_at) * 1000
            parameters = json.dumps({"path": path, "actor": actor})
            try:
                await db.execute(
                    """
                    INSERT INTO tool_calls (name, parameters, result, error, status, started_at, completed_at, duration_ms)
                    VALUES (?, ?, ?, ?, ?, ?, ?, ?)
                    """,
                    (name, parameters, result_summary, error_msg, status, started_at, completed_at, duration_ms),
                )
            except Exception as e:
                logger = __import__("logging").getLogger(__name__)
                logger.debug("audit: failed to insert tool_calls row on flush: %s", e)
        await db.commit_transaction()
        await db.close()

    async def abort(self) -> None:
        """Discard all changes and close the task connection."""
        if self._closed:
            return
        self._closed = True
        await self._db.rollback()
        await self._db.close()

    def get_database(self) -> DbConnection:
        return self._db

    async def __aenter__(self) -> "Task":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        if not self._closed:
            await self.abort()


class SecAFS:
    """SecAFS - A filesystem and key-value store for AI agents (PostgreSQL backend)

    Provides a unified interface for persistent storage using PostgreSQL,
    with support for key-value storage, filesystem operations, and
    tool call tracking.
    """

    def __init__(
        self,
        db: DbConnection,
        kv: KvStore,
        fs: Filesystem,
        tools: ToolCalls,
        *,
        _connect_url: Optional[str] = None,
    ):
        self._db = db
        self.kv = kv
        self.fs = fs
        self.tools = tools
        self._connect_url = _connect_url

    @staticmethod
    async def open(options: SecAFSOptions) -> "SecAFS":
        """Open an agent filesystem

        Args:
            options: Configuration options with postgres_url

        Returns:
            Fully initialized SecAFS instance
        """
        if not options.postgres_url:
            raise ValueError("SecAFS.open() requires 'postgres_url'.")

        db = await connect_postgres(options.postgres_url, backend=options.backend)
        instance = await SecAFS.open_with(db, skip_schema_init=getattr(options, "skip_schema_init", False))
        instance._connect_url = options.postgres_url
        return instance

    @staticmethod
    async def open_with(db: DbConnection, skip_schema_init: bool = False) -> "SecAFS":
        """Open a SecAFS instance with an existing database connection

        Args:
            db: An existing database connection
            skip_schema_init: If True, do not run CREATE/ALTER (for limited DB roles)

        Returns:
            Fully initialized SecAFS instance
        """
        kv = await KvStore.from_database(db, skip_schema_init=skip_schema_init)
        fs = await Filesystem.from_database(db, skip_schema_init=skip_schema_init)
        tools = await ToolCalls.from_database(db, skip_schema_init=skip_schema_init)

        return SecAFS(db, kv, fs, tools)

    async def begin_task(self) -> "Task":
        """Start a transactional task backed by a dedicated connection.

        The returned ``Task`` object holds its own database connection with an
        open transaction.  All filesystem / KV / tool-call operations performed
        through the task are invisible to other connections until
        ``task.commit()`` is called.

        Returns:
            A new ``Task`` instance with ``fs``, ``kv`` and ``tools``
            attributes ready to use.
        """
        if not self._connect_url:
            raise RuntimeError(
                "Cannot begin_task: connection URL was not recorded. "
                "Open the SecAFS instance via SecAFS.open() first."
            )

        db = await connect_postgres(self._connect_url)

        task = Task(db, connect_url=self._connect_url)
        await task._init_components()
        await db.begin()
        return task

    def get_database(self) -> DbConnection:
        """Get the underlying Database connection"""
        return self._db

    async def close(self) -> None:
        """Close the database connection"""
        await self._db.close()

    async def __aenter__(self) -> "SecAFS":
        return self

    async def __aexit__(self, exc_type, exc, tb) -> None:
        await self.close()
