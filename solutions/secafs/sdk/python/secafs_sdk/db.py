"""Database abstraction for SecAFS SDK (PostgreSQL and OpenGauss)."""

from __future__ import annotations

import re
from dataclasses import dataclass, field
from typing import Any, List, Optional, Sequence, Tuple

import asyncpg

# ---------------------------------------------------------------------------
# OpenGauss SQL compatibility helpers
# ---------------------------------------------------------------------------
# OpenGauss 5.0/6.0 (based on PG 9.2 kernel) does NOT support the PostgreSQL
# ``ON CONFLICT ... DO UPDATE SET col = EXCLUDED.col`` syntax (PG 9.5+).
# It does support MySQL-compatible ``ON DUPLICATE KEY UPDATE col = VALUES(col)``.
# The helpers below transparently rewrite the SQL at execution time so that
# all existing SecAFS queries work unmodified on OpenGauss.

_ON_CONFLICT_DO_UPDATE_RE = re.compile(
    r"\bON\s+CONFLICT\s*(?:\([^)]*\))?\s*DO\s+UPDATE\s+SET\s+",
    re.IGNORECASE,
)
_ON_CONFLICT_DO_NOTHING_RE = re.compile(
    r"\bON\s+CONFLICT\s*(?:\([^)]*\))?\s*DO\s+NOTHING\b",
    re.IGNORECASE,
)
_EXCLUDED_REF_RE = re.compile(r"\bEXCLUDED\.(\w+)", re.IGNORECASE)


def _rewrite_on_conflict_for_opengauss(sql: str) -> str:
    """Rewrite PostgreSQL ``ON CONFLICT ... DO UPDATE`` to OpenGauss-compatible syntax.

    * ``ON CONFLICT (...) DO UPDATE SET col = EXCLUDED.col``
      → ``ON DUPLICATE KEY UPDATE col = VALUES(col)``
    * ``ON CONFLICT (...) DO NOTHING``
      → removed entirely (the caller should catch unique-violation instead)
    """
    # Handle DO NOTHING first (just strip the clause)
    sql = _ON_CONFLICT_DO_NOTHING_RE.sub("", sql)
    # Handle DO UPDATE SET ...
    if _ON_CONFLICT_DO_UPDATE_RE.search(sql):
        sql = _ON_CONFLICT_DO_UPDATE_RE.sub("ON DUPLICATE KEY UPDATE ", sql)
        sql = _EXCLUDED_REF_RE.sub(r"VALUES(\1)", sql)
    return sql


class DbCursor:
    def __init__(self, rows: Optional[List[asyncpg.Record]] = None):
        self._rows = rows or []
        self._index = 0

    async def fetchone(self) -> Optional[Tuple[Any, ...]]:
        if self._index >= len(self._rows):
            return None
        row = self._rows[self._index]
        self._index += 1
        return tuple(row)

    async def fetchall(self) -> List[Tuple[Any, ...]]:
        return [tuple(row) for row in self._rows]


@dataclass
class DbConnection:
    conn: Any
    backend: str = field(default="postgres")

    async def execute(self, sql: str, params: Sequence[Any] = ()) -> DbCursor:
        original_sql = sql
        sql, params = rebind_sql(sql, params)
        if self.backend == "opengauss":
            sql = _rewrite_on_conflict_for_opengauss(sql)
        try:
            rows = await self.conn.fetch(sql, *params)
        except asyncpg.UniqueViolationError:
            # If the original SQL contained ON CONFLICT ... DO NOTHING which was
            # stripped for OpenGauss compatibility, silently ignore unique violations.
            if self.backend == "opengauss" and _ON_CONFLICT_DO_NOTHING_RE.search(original_sql):
                return DbCursor(rows=[])
            raise
        return DbCursor(rows=rows)

    async def executescript(self, script: str) -> None:
        statements = [s.strip() for s in script.split(";") if s.strip()]
        for stmt in statements:
            await self.execute(stmt)

    async def commit(self) -> None:
        pass

    async def run_command(self, sql: str) -> None:
        """Execute a transaction-control statement (BEGIN / SAVEPOINT / ROLLBACK / COMMIT).

        Unlike ``execute``, this does not try to fetch result rows and works
        correctly for statements that return no data.
        """
        await self.conn.execute(sql)

    async def begin(self) -> None:
        await self.run_command("BEGIN")

    async def savepoint(self, name: str) -> None:
        await self.run_command(f"SAVEPOINT {name}")

    async def rollback_to_savepoint(self, name: str) -> None:
        await self.run_command(f"ROLLBACK TO SAVEPOINT {name}")

    async def rollback(self) -> None:
        await self.run_command("ROLLBACK")

    async def commit_transaction(self) -> None:
        await self.run_command("COMMIT")

    async def close(self) -> None:
        await self.conn.close()


def normalize_db_url(url: str) -> str:
    """Normalize opengauss:// URLs to postgres:// for driver compatibility."""
    if url.startswith("opengauss://"):
        return url.replace("opengauss://", "postgres://", 1)
    return url


async def detect_backend(conn: DbConnection) -> str:
    """Detect the database backend by querying the version string.

    Returns:
        ``"opengauss"`` if the server identifies as openGauss,
        ``"postgres"`` otherwise.
    """
    cursor = await conn.execute("SELECT version()", ())
    row = await cursor.fetchone()
    if row and "opengauss" in str(row[0]).lower():
        return "opengauss"
    return "postgres"


async def connect_postgres(url: str, backend: Optional[str] = None) -> DbConnection:
    """Connect to a PostgreSQL or OpenGauss database.

    ``opengauss://`` URLs are automatically normalized to ``postgres://``
    for driver compatibility.  When *backend* is not given explicitly, it
    is inferred from the URL scheme (``opengauss://`` → ``"opengauss"``).
    """
    if backend is None:
        backend = "opengauss" if url.startswith("opengauss://") else "postgres"
    url = normalize_db_url(url)
    conn = await asyncpg.connect(url)
    return DbConnection(conn=conn, backend=backend)


def rebind_sql(sql: str, params: Sequence[Any]) -> Tuple[str, Sequence[Any]]:
    out = []
    index = 1
    i = 0
    while i < len(sql):
        ch = sql[i]
        if ch == "?":
            j = i + 1
            while j < len(sql) and sql[j].isdigit():
                j += 1
            out.append(f"${index}")
            index += 1
            i = j
            continue
        out.append(ch)
        i += 1
    return "".join(out), params
