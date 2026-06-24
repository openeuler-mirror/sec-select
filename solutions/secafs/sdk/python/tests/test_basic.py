"""Basic tests for SecAFS Python SDK"""

import time

import pytest

from secafs_sdk import SecAFS


@pytest.mark.asyncio
async def test_secafs_open_with_connection(db):
    """Test opening SecAFS from an existing connection"""
    secafs_inst = await SecAFS.open_with(db)
    assert secafs_inst is not None
    assert secafs_inst.kv is not None
    assert secafs_inst.fs is not None
    assert secafs_inst.tools is not None


@pytest.mark.asyncio
async def test_kvstore_basic(db):
    """Test basic key-value operations"""
    secafs_inst = await SecAFS.open_with(db)

    # Set and get
    await secafs_inst.kv.set("test_key", "test_value")
    value = await secafs_inst.kv.get("test_key")
    assert value == "test_value"

    # Set complex object
    obj = {"name": "Alice", "age": 30}
    await secafs_inst.kv.set("user", obj)
    retrieved = await secafs_inst.kv.get("user")
    assert retrieved == obj

    # Delete
    await secafs_inst.kv.delete("test_key")
    value = await secafs_inst.kv.get("test_key")
    assert value is None


@pytest.mark.asyncio
async def test_filesystem_basic(db):
    """Test basic filesystem operations"""
    secafs_inst = await SecAFS.open_with(db)

    # Write and read file
    await secafs_inst.fs.write_file("/test.txt", "Hello, World!")
    content = await secafs_inst.fs.read_file("/test.txt")
    assert content == "Hello, World!"

    # Create nested file (auto-create parent dirs)
    await secafs_inst.fs.write_file("/dir1/dir2/file.txt", "nested")
    content = await secafs_inst.fs.read_file("/dir1/dir2/file.txt")
    assert content == "nested"

    # List directory
    files = await secafs_inst.fs.readdir("/dir1")
    assert "dir2" in files

    # Get stats
    stats = await secafs_inst.fs.stat("/test.txt")
    assert stats.is_file()
    assert stats.size == len("Hello, World!")


@pytest.mark.asyncio
async def test_toolcalls_basic(db):
    """Test basic tool call tracking"""
    secafs_inst = await SecAFS.open_with(db)

    # Record a tool call
    start = int(time.time())
    end = start + 1

    call_id = await secafs_inst.tools.record(
        "test_tool", start, end, parameters={"param": "value"}, result={"result": "success"}
    )

    assert call_id > 0

    # Get the tool call
    call = await secafs_inst.tools.get(call_id)
    assert call is not None
    assert call.name == "test_tool"
    assert call.parameters == {"param": "value"}
    assert call.result == {"result": "success"}
    assert call.status == "success"

    # Get stats
    stats = await secafs_inst.tools.get_stats()
    assert len(stats) == 1
    assert stats[0].name == "test_tool"
    assert stats[0].total_calls == 1
