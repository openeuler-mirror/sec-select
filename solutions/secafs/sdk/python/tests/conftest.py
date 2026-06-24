"""Shared pytest fixtures for the SecAFS SDK test suite.

Tests run against a real PostgreSQL / openGauss backend. The connection URL is
read from the ``SECAFS_TEST_POSTGRES_URL`` environment variable, falling back to
the local development database defined in ``docker-compose.dev.yml`` (the
``opengauss`` service). When no database is reachable — or the database driver
is not installed — the affected tests are skipped rather than failed, so the
suite degrades gracefully in environments without a database.

Each test gets an isolated, freshly created schema set as the connection's
``search_path``. The SDK creates all of its tables with ``CREATE TABLE IF NOT
EXISTS`` and unqualified names, so they land in that per-test schema. The schema
is dropped on teardown, guaranteeing a clean slate between tests.
"""

import os
import uuid

import pytest

# Default to the local dev openGauss instance from docker-compose.dev.yml.
DEFAULT_TEST_URL = "opengauss://secafs:Secafs!123@localhost:5433/secafs"


@pytest.fixture
async def db():
    """Yield a clean ``DbConnection`` backed by a unique per-test schema.

    Skips the test when the database driver is missing or no database is
    reachable.
    """
    try:
        from secafs_sdk.db import connect_postgres
    except ImportError:
        pytest.skip("database driver (asyncpg) not installed")

    url = os.environ.get("SECAFS_TEST_POSTGRES_URL", DEFAULT_TEST_URL)

    try:
        connection = await connect_postgres(url)
    except Exception:
        pytest.skip("no SecAFS test database reachable")

    schema = f"secafs_test_{uuid.uuid4().hex}"
    try:
        await connection.run_command(f'CREATE SCHEMA "{schema}"')
        await connection.run_command(f'SET search_path TO "{schema}"')
        yield connection
    finally:
        try:
            await connection.run_command(f'DROP SCHEMA IF EXISTS "{schema}" CASCADE')
        finally:
            await connection.close()
