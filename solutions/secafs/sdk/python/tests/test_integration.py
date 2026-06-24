"""SecAFS Integration Tests"""

import pytest

from secafs_sdk import SecAFS


@pytest.mark.asyncio
class TestSecAFSIntegration:
    """Integration tests for the unified SecAFS facade"""

    async def test_open_with_initializes_components(self, db):
        """Should initialize kv, fs and tools components from a connection"""
        instance = await SecAFS.open_with(db)
        assert instance is not None
        assert isinstance(instance, SecAFS)
        assert instance.kv is not None
        assert instance.fs is not None
        assert instance.tools is not None

    async def test_get_database_returns_connection(self, db):
        """Should return the underlying database connection"""
        instance = await SecAFS.open_with(db)
        assert instance.get_database() is db

    async def test_kv_round_trip(self, db):
        """Should persist and read back a key-value pair"""
        instance = await SecAFS.open_with(db)

        await instance.kv.set("test", "value")
        value = await instance.kv.get("test")
        assert value == "value"

    async def test_components_share_connection(self, db):
        """Should expose fs, kv and tools backed by the same connection"""
        instance = await SecAFS.open_with(db)

        await instance.fs.write_file("/note.txt", "hello")
        assert await instance.fs.read_file("/note.txt") == "hello"

        call_id = await instance.tools.start("probe", {"arg": 1})
        assert call_id > 0

    async def test_data_visible_across_facades(self, db):
        """Should see data written through one facade from another on the same connection"""
        first = await SecAFS.open_with(db)
        await first.kv.set("shared", "value1")

        second = await SecAFS.open_with(db)
        value = await second.kv.get("shared")
        assert value == "value1"
