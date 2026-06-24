"""KvStore Integration Tests"""

import pytest

from secafs_sdk import KvStore


@pytest.mark.asyncio
class TestKvStoreBasicOperations:
    """Basic KvStore operations"""

    async def test_set_and_get_string(self, db):
        """Should set and get a string value"""
        kv = await KvStore.from_database(db)

        await kv.set("test-key", "test-value")
        value = await kv.get("test-key")
        assert value == "test-value"

    async def test_set_and_get_object(self, db):
        """Should set and get an object value"""
        kv = await KvStore.from_database(db)

        test_object = {"name": "test", "count": 42, "nested": {"value": True}}
        await kv.set("object-key", test_object)
        value = await kv.get("object-key")
        assert value == test_object

    async def test_set_and_get_number(self, db):
        """Should set and get a number value"""
        kv = await KvStore.from_database(db)

        await kv.set("number-key", 12345)
        value = await kv.get("number-key")
        assert value == 12345

    async def test_set_and_get_boolean(self, db):
        """Should set and get a boolean value"""
        kv = await KvStore.from_database(db)

        await kv.set("bool-key", True)
        value = await kv.get("bool-key")
        assert value is True

    async def test_set_and_get_array(self, db):
        """Should set and get an array value"""
        kv = await KvStore.from_database(db)

        test_array = [1, 2, "three", {"four": 4}]
        await kv.set("array-key", test_array)
        value = await kv.get("array-key")
        assert value == test_array

    async def test_set_and_list_values(self, db):
        """Should set and list values"""
        kv = await KvStore.from_database(db)

        await kv.set("g1:k1", 1)
        await kv.set("g1:k2", 2)
        await kv.set("g2:k1", 3)
        await kv.set("g2:k2", 4)

        result1 = await kv.list("g1:")
        assert result1 == [{"key": "g1:k1", "value": 1}, {"key": "g1:k2", "value": 2}]

        result2 = await kv.list("g1:k1")
        assert result2 == [{"key": "g1:k1", "value": 1}]

        result3 = await kv.list("g1:k3")
        assert result3 == []

        result4 = await kv.list("g2:")
        assert result4 == [{"key": "g2:k1", "value": 3}, {"key": "g2:k2", "value": 4}]


@pytest.mark.asyncio
class TestKvStoreUpdateOperations:
    """KvStore update operations"""

    async def test_update_existing_value(self, db):
        """Should update an existing value"""
        kv = await KvStore.from_database(db)

        await kv.set("update-key", "initial-value")
        await kv.set("update-key", "updated-value")
        value = await kv.get("update-key")
        assert value == "updated-value"

    async def test_update_value_type(self, db):
        """Should update value type"""
        kv = await KvStore.from_database(db)

        await kv.set("type-key", "string-value")
        await kv.set("type-key", {"object": "value"})
        value = await kv.get("type-key")
        assert value == {"object": "value"}


@pytest.mark.asyncio
class TestKvStoreDeleteOperations:
    """KvStore delete operations"""

    async def test_delete_existing_key(self, db):
        """Should delete an existing key"""
        kv = await KvStore.from_database(db)

        await kv.set("delete-key", "value-to-delete")
        await kv.delete("delete-key")
        value = await kv.get("delete-key")
        assert value is None

    async def test_delete_nonexistent_key(self, db):
        """Should handle deleting non-existent key"""
        kv = await KvStore.from_database(db)

        # Should not throw an error when deleting a non-existent key
        await kv.delete("non-existent-key")


@pytest.mark.asyncio
class TestKvStoreEdgeCases:
    """KvStore edge cases"""

    async def test_get_nonexistent_key(self, db):
        """Should return None for non-existent key"""
        kv = await KvStore.from_database(db)

        value = await kv.get("non-existent-key")
        assert value is None

    async def test_get_with_default(self, db):
        """Should return default value for non-existent key"""
        kv = await KvStore.from_database(db)

        value = await kv.get("non-existent-key", default="default-value")
        assert value == "default-value"

    async def test_handle_null_values(self, db):
        """Should handle null values"""
        kv = await KvStore.from_database(db)

        await kv.set("null-key", None)
        value = await kv.get("null-key")
        assert value is None

    async def test_handle_empty_string(self, db):
        """Should handle empty string"""
        kv = await KvStore.from_database(db)

        await kv.set("empty-key", "")
        value = await kv.get("empty-key")
        assert value == ""

    async def test_handle_zero_value(self, db):
        """Should handle zero value"""
        kv = await KvStore.from_database(db)

        await kv.set("zero-key", 0)
        value = await kv.get("zero-key")
        assert value == 0

    async def test_keys_with_special_characters(self, db):
        """Should handle keys with special characters"""
        kv = await KvStore.from_database(db)

        special_key = "key:with/special.chars@123"
        await kv.set(special_key, "value")
        value = await kv.get(special_key)
        assert value == "value"


@pytest.mark.asyncio
class TestKvStoreLargeData:
    """KvStore large data tests"""

    async def test_large_string_values(self, db):
        """Should handle large string values"""
        kv = await KvStore.from_database(db)

        large_string = "x" * 10000
        await kv.set("large-string", large_string)
        value = await kv.get("large-string")
        assert value == large_string

    async def test_large_object_values(self, db):
        """Should handle large object values"""
        kv = await KvStore.from_database(db)

        large_object = {
            "items": [
                {"id": i, "name": f"Item {i}", "data": f"Data for item {i}"}
                for i in range(1000)
            ]
        }
        await kv.set("large-object", large_object)
        value = await kv.get("large-object")
        assert value == large_object


@pytest.mark.asyncio
class TestKvStorePersistence:
    """KvStore persistence tests"""

    async def test_persist_across_instances(self, db):
        """Should persist data across KvStore instances"""
        kv = await KvStore.from_database(db)

        await kv.set("persist-key", "persist-value")

        # Create new KvStore instance with same database
        new_kv = await KvStore.from_database(db)
        value = await new_kv.get("persist-key")
        assert value == "persist-value"
