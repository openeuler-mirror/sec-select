"""ToolCalls Integration Tests"""

import asyncio
import time

import pytest

from secafs_sdk import ToolCalls


@pytest.mark.asyncio
class TestToolCallsBasicOperations:
    """Basic ToolCalls operations"""

    async def test_start_tool_call_returns_id(self, db):
        """Should start a tool call and return an ID"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("test_tool", {"arg1": "value1"})
        assert call_id > 0

    async def test_start_without_parameters(self, db):
        """Should start a tool call without parameters"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("simple_tool")
        assert call_id > 0

        tool_call = await tools.get(call_id)
        assert tool_call is not None
        assert tool_call.name == "simple_tool"
        assert tool_call.parameters is None
        assert tool_call.status == "pending"

    async def test_mark_call_as_successful(self, db):
        """Should mark a tool call as successful"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("test_tool", {"input": "test"})
        await tools.success(call_id, {"output": "result"})

        tool_call = await tools.get(call_id)
        assert tool_call is not None
        assert tool_call.status == "success"
        assert tool_call.result == {"output": "result"}
        assert tool_call.completed_at is not None and tool_call.completed_at > 0
        assert tool_call.duration_ms is not None and tool_call.duration_ms >= 0

    async def test_mark_successful_without_result(self, db):
        """Should mark a tool call as successful without result"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("test_tool", {"input": "test"})
        await tools.success(call_id)

        tool_call = await tools.get(call_id)
        assert tool_call is not None
        assert tool_call.status == "success"
        assert tool_call.result is None

    async def test_mark_call_as_failed(self, db):
        """Should mark a tool call as failed"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("test_tool", {"input": "test"})
        await tools.error(call_id, "Something went wrong")

        tool_call = await tools.get(call_id)
        assert tool_call is not None
        assert tool_call.status == "error"
        assert tool_call.error == "Something went wrong"
        assert tool_call.completed_at is not None and tool_call.completed_at > 0
        assert tool_call.duration_ms is not None and tool_call.duration_ms >= 0

    async def test_get_tool_call_by_id(self, db):
        """Should get a tool call by ID"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("test_tool", {"arg": "value"})
        tool_call = await tools.get(call_id)

        assert tool_call is not None
        assert tool_call.id == call_id
        assert tool_call.name == "test_tool"
        assert tool_call.parameters == {"arg": "value"}
        assert tool_call.status == "pending"
        assert tool_call.started_at > 0

    async def test_get_nonexistent_id(self, db):
        """Should return None for non-existent ID"""
        tools = await ToolCalls.from_database(db)

        tool_call = await tools.get(99999)
        assert tool_call is None


@pytest.mark.asyncio
class TestToolCallsQueryOperations:
    """ToolCalls query operations"""

    async def test_get_by_name(self, db):
        """Should get tool calls by name"""
        tools = await ToolCalls.from_database(db)

        await tools.start("tool_a", {"test": 1})
        await tools.start("tool_b", {"test": 2})
        await tools.start("tool_a", {"test": 3})

        results = await tools.get_by_name("tool_a")
        assert len(results) == 2
        assert all(tc.name == "tool_a" for tc in results)

    async def test_limit_results_by_name(self, db):
        """Should limit results when querying by name"""
        tools = await ToolCalls.from_database(db)

        await tools.start("tool_a", {"test": 1})
        await tools.start("tool_a", {"test": 2})
        await tools.start("tool_a", {"test": 3})

        results = await tools.get_by_name("tool_a", limit=2)
        assert len(results) == 2

    async def test_get_recent_calls(self, db):
        """Should get recent tool calls"""
        tools = await ToolCalls.from_database(db)

        await tools.start("tool_old")
        # Wait to ensure different timestamps
        await asyncio.sleep(1.1)
        midpoint = int(time.time())
        await asyncio.sleep(1.1)
        await tools.start("tool_new")

        results = await tools.get_recent(midpoint)
        assert len(results) >= 1
        assert all(tc.started_at >= midpoint for tc in results)

    async def test_limit_recent_calls(self, db):
        """Should limit recent tool calls"""
        tools = await ToolCalls.from_database(db)

        now = int(time.time())

        await tools.start("tool_1")
        await tools.start("tool_2")
        await tools.start("tool_3")

        results = await tools.get_recent(now - 10, limit=2)
        assert len(results) <= 2

    async def test_empty_results_for_nonexistent_name(self, db):
        """Should return empty array when no matching tool calls by name"""
        tools = await ToolCalls.from_database(db)

        results = await tools.get_by_name("non_existent_tool")
        assert results == []


@pytest.mark.asyncio
class TestToolCallsStatistics:
    """ToolCalls statistics"""

    async def test_calculate_statistics(self, db):
        """Should calculate tool call statistics"""
        tools = await ToolCalls.from_database(db)

        # Create some tool calls
        id1 = await tools.start("tool_a", {"test": 1})
        await tools.success(id1, {"result": "ok"})

        id2 = await tools.start("tool_a", {"test": 2})
        await tools.error(id2, "failed")

        id3 = await tools.start("tool_a", {"test": 3})
        await tools.success(id3, {"result": "ok"})

        id4 = await tools.start("tool_b", {"test": 4})
        await tools.success(id4, {"result": "ok"})

        stats = await tools.get_stats()

        assert len(stats) == 2

        tool_a_stats = next((s for s in stats if s.name == "tool_a"), None)
        assert tool_a_stats is not None
        assert tool_a_stats.total_calls == 3
        assert tool_a_stats.successful == 2
        assert tool_a_stats.failed == 1
        assert tool_a_stats.avg_duration_ms >= 0

        tool_b_stats = next((s for s in stats if s.name == "tool_b"), None)
        assert tool_b_stats is not None
        assert tool_b_stats.total_calls == 1
        assert tool_b_stats.successful == 1
        assert tool_b_stats.failed == 0

    async def test_exclude_pending_from_stats(self, db):
        """Should exclude pending calls from statistics"""
        tools = await ToolCalls.from_database(db)

        id1 = await tools.start("tool_a", {"test": 1})
        await tools.success(id1, {"result": "ok"})

        # This one stays pending
        await tools.start("tool_a", {"test": 2})

        stats = await tools.get_stats()
        tool_a_stats = next((s for s in stats if s.name == "tool_a"), None)

        assert tool_a_stats is not None
        assert tool_a_stats.total_calls == 1  # Only completed calls

    async def test_empty_stats_no_completed_calls(self, db):
        """Should return empty array when no completed calls"""
        tools = await ToolCalls.from_database(db)

        await tools.start("tool_a", {"test": 1})
        stats = await tools.get_stats()
        assert stats == []


@pytest.mark.asyncio
class TestToolCallsComplexData:
    """ToolCalls complex parameters and results"""

    async def test_complex_nested_parameters(self, db):
        """Should handle complex nested parameters"""
        tools = await ToolCalls.from_database(db)

        complex_params = {
            "user": {"id": 123, "name": "Test User"},
            "options": {"timeout": 5000, "retry": True},
            "data": [1, 2, 3, 4, 5],
        }

        call_id = await tools.start("complex_tool", complex_params)
        tool_call = await tools.get(call_id)

        assert tool_call is not None
        assert tool_call.parameters == complex_params

    async def test_complex_nested_results(self, db):
        """Should handle complex nested results"""
        tools = await ToolCalls.from_database(db)

        complex_result = {
            "data": {"items": [{"id": 1, "value": "a"}, {"id": 2, "value": "b"}]},
            "metadata": {"count": 2, "hasMore": False},
        }

        call_id = await tools.start("complex_tool")
        await tools.success(call_id, complex_result)
        tool_call = await tools.get(call_id)

        assert tool_call is not None
        assert tool_call.result == complex_result

    async def test_large_parameters(self, db):
        """Should handle large parameters"""
        tools = await ToolCalls.from_database(db)

        large_params = {"items": [{"id": i, "data": f"Data for item {i}"} for i in range(100)]}

        call_id = await tools.start("large_tool", large_params)
        tool_call = await tools.get(call_id)

        assert tool_call is not None
        assert tool_call.parameters == large_params


@pytest.mark.asyncio
class TestToolCallsPersistence:
    """ToolCalls persistence"""

    async def test_persist_across_instances(self, db):
        """Should persist tool calls across instances"""
        tools = await ToolCalls.from_database(db)

        call_id = await tools.start("persist_tool", {"test": "value"})
        await tools.success(call_id, {"result": "ok"})

        # Create new ToolCalls instance with same database
        new_tools = await ToolCalls.from_database(db)
        tool_call = await new_tools.get(call_id)

        assert tool_call is not None
        assert tool_call.name == "persist_tool"
        assert tool_call.status == "success"


@pytest.mark.asyncio
class TestToolCallsOrdering:
    """ToolCalls ordering"""

    async def test_order_by_started_at_desc(self, db):
        """Should return tool calls ordered by started_at desc"""
        tools = await ToolCalls.from_database(db)

        await tools.start("tool_first")
        await asyncio.sleep(0.1)
        await tools.start("tool_second")
        await asyncio.sleep(0.1)
        await tools.start("tool_third")

        recent = await tools.get_recent(0)

        assert len(recent) >= 3
        # Most recent first
        for i in range(len(recent) - 1):
            assert recent[i].started_at >= recent[i + 1].started_at


@pytest.mark.asyncio
class TestToolCallsRecord:
    """ToolCalls record method"""

    async def test_record_completed_call(self, db):
        """Should record a completed tool call"""
        tools = await ToolCalls.from_database(db)

        start = int(time.time())
        end = start + 1

        call_id = await tools.record(
            "test_tool",
            start,
            end,
            parameters={"param": "value"},
            result={"result": "success"},
        )

        assert call_id > 0

        # Get the tool call
        call = await tools.get(call_id)
        assert call is not None
        assert call.name == "test_tool"
        assert call.parameters == {"param": "value"}
        assert call.result == {"result": "success"}
        assert call.status == "success"

    async def test_record_failed_call(self, db):
        """Should record a failed tool call"""
        tools = await ToolCalls.from_database(db)

        start = int(time.time())
        end = start + 1

        call_id = await tools.record(
            "test_tool", start, end, parameters={"param": "value"}, error="Failed"
        )

        assert call_id > 0

        # Get the tool call
        call = await tools.get(call_id)
        assert call is not None
        assert call.name == "test_tool"
        assert call.error == "Failed"
        assert call.status == "error"
