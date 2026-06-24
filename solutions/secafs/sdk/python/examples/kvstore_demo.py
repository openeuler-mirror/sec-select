"""Key-Value Store example for SecAFS Python SDK"""

import asyncio
import json
import time

from secafs_sdk import SecAFS, SecAFSOptions


async def main():
    # Initialize SecAFS with persistent storage
    secafs_inst = await SecAFS.open(SecAFSOptions(id="kvstore-demo"))

    print("=== KvStore Example ===\n")

    # Example 1: Store and retrieve simple values
    print("1. Storing simple values:")
    await secafs_inst.kv.set("username", "alice")
    await secafs_inst.kv.set("age", 30)
    await secafs_inst.kv.set("active", True)

    username = await secafs_inst.kv.get("username")
    age = await secafs_inst.kv.get("age")
    active = await secafs_inst.kv.get("active")

    print(f"  Username: {username}")
    print(f"  Age: {age}")
    print(f"  Active: {active}\n")

    # Example 2: Store and retrieve objects
    print("2. Storing complex objects:")
    user = {
        "id": 1,
        "name": "Alice Johnson",
        "email": "alice@example.com",
        "preferences": {"theme": "dark", "notifications": True},
    }

    await secafs_inst.kv.set("user:1", user)
    retrieved_user = await secafs_inst.kv.get("user:1")
    print(f"  Stored user: {json.dumps(retrieved_user, indent=2)}\n")

    # Example 3: Store and retrieve arrays
    print("3. Storing arrays:")
    tags = ["python", "database", "ai", "agent"]
    await secafs_inst.kv.set("tags", tags)
    retrieved_tags = await secafs_inst.kv.get("tags")
    assert isinstance(retrieved_tags, list)
    print(f"  Tags: {', '.join(retrieved_tags)}\n")

    # Example 4: Update existing values
    print("4. Updating existing values:")
    print(f"  Age before update: {await secafs_inst.kv.get('age')}")
    await secafs_inst.kv.set("age", 31)
    print(f"  Age after update: {await secafs_inst.kv.get('age')}\n")

    # Example 5: Delete values
    print("5. Deleting values:")
    print(f"  Username before delete: {await secafs_inst.kv.get('username')}")
    await secafs_inst.kv.delete("username")
    print(f"  Username after delete: {await secafs_inst.kv.get('username')}\n")

    # Example 6: Handle non-existent keys
    print("6. Retrieving non-existent keys:")
    non_existent = await secafs_inst.kv.get("does-not-exist")
    print(f"  Result: {non_existent}\n")

    # Example 7: Use cases for AI agents
    print("7. AI Agent use cases:")

    # Session state
    await secafs_inst.kv.set(
        "session:current",
        {"conversationId": "conv-123", "userId": "user-456", "startTime": int(time.time() * 1000)},
    )

    # Agent memory
    await secafs_inst.kv.set(
        "memory:user-preferences",
        {"language": "en", "responseStyle": "concise", "expertise": "intermediate"},
    )

    # Task queue
    await secafs_inst.kv.set(
        "tasks:pending",
        [
            {"id": 1, "task": "Process document", "priority": "high"},
            {"id": 2, "task": "Send notification", "priority": "low"},
        ],
    )

    print(f"  Session: {json.dumps(await secafs_inst.kv.get('session:current'), indent=2)}")
    print(f"  Memory: {json.dumps(await secafs_inst.kv.get('memory:user-preferences'), indent=2)}")
    print(f"  Tasks: {json.dumps(await secafs_inst.kv.get('tasks:pending'), indent=2)}")

    print("\n=== Example Complete ===")

    # Close the database
    await secafs_inst.close()


if __name__ == "__main__":
    asyncio.run(main())
