# SecAFS Python SDK

A filesystem and key-value store for AI agents, powered by PostgreSQL/openGauss.

## Installation

```bash
pip install secafs-sdk
```

## Quick Start

```python
import asyncio
from secafs_sdk import SecAFS, SecAFSOptions

async def main():
    # Open an agent filesystem backed by PostgreSQL/openGauss
    agent = await SecAFS.open(SecAFSOptions(postgres_url='postgres://localhost/secafs'))

    # Use key-value store
    await agent.kv.set('config', {'debug': True, 'version': '1.0'})
    config = await agent.kv.get('config')
    print(f"Config: {config}")

    # Use filesystem
    await agent.fs.write_file('/data/notes.txt', 'Hello, SecAFS!')
    content = await agent.fs.read_file('/data/notes.txt')
    print(f"Content: {content}")

    # Track tool calls
    call_id = await agent.tools.start('search', {'query': 'Python'})
    await agent.tools.success(call_id, {'results': ['result1', 'result2']})

    # Get statistics
    stats = await agent.tools.get_stats()
    for stat in stats:
        print(f"{stat.name}: {stat.total_calls} calls, {stat.avg_duration_ms:.2f}ms avg")

    # Close the database
    await agent.close()

if __name__ == '__main__':
    asyncio.run(main())
```

## openGauss / PostgreSQL Backend

Use an openGauss (or PostgreSQL) connection URL for remote or shared deployments. The
`opengauss://` scheme is normalized to `postgres://` for the driver, so stock
PostgreSQL works too:

```python
agent = await SecAFS.open(SecAFSOptions(postgres_url="opengauss://user:pass@host:5432/db"))
```

## Features

### Key-Value Store

Simple key-value storage with JSON serialization:

```python
# Set a value
await agent.kv.set('user:123', {'name': 'Alice', 'age': 30})

# Get a value
user = await agent.kv.get('user:123')

# List by prefix
users = await agent.kv.list('user:')

# Delete a value
await agent.kv.delete('user:123')
```

### Filesystem

POSIX-like filesystem operations:

```python
# Write a file (creates parent directories automatically)
await agent.fs.write_file('/data/config.json', '{"key": "value"}')

# Read a file
content = await agent.fs.read_file('/data/config.json')

# Read as bytes
data = await agent.fs.read_file('/data/image.png', encoding=None)

# List directory
entries = await agent.fs.readdir('/data')

# Get file stats
stats = await agent.fs.stat('/data/config.json')
print(f"Size: {stats.size} bytes")
print(f"Modified: {stats.mtime}")
print(f"Is file: {stats.is_file()}")

# Delete a file
await agent.fs.delete_file('/data/config.json')
```

### Tool Calls Tracking

Track and analyze tool/function calls:

```python
# Start a tool call
call_id = await agent.tools.start('search', {'query': 'Python'})

# Mark as successful
await agent.tools.success(call_id, {'results': [...]})

# Or mark as failed
await agent.tools.error(call_id, 'Connection timeout')

# Record a completed call
await agent.tools.record(
    'search',
    started_at=1234567890,
    completed_at=1234567892,
    parameters={'query': 'Python'},
    result={'results': [...]}
)

# Query tool calls
calls = await agent.tools.get_by_name('search', limit=10)
recent = await agent.tools.get_recent(since=1234567890)

# Get statistics
stats = await agent.tools.get_stats()
for stat in stats:
    print(f"{stat.name}: {stat.successful}/{stat.total_calls} successful")
```

## Configuration

### Connection URL

`SecAFS.open()` takes a `SecAFSOptions` with a PostgreSQL or openGauss
connection URL. The `opengauss://` scheme is auto-detected and normalized to
`postgres://` for the driver:

```python
# PostgreSQL
agent = await SecAFS.open(SecAFSOptions(postgres_url='postgres://user:pass@localhost:5432/secafs'))

# openGauss
agent = await SecAFS.open(SecAFSOptions(postgres_url='opengauss://user:pass@host:5432/secafs'))
```

### Limited Database Roles

When the connecting role only has `SELECT`/`INSERT`/`UPDATE`/`DELETE` on
existing tables (no `CREATE`/`ALTER`), skip schema initialization:

```python
agent = await SecAFS.open(SecAFSOptions(
    postgres_url='postgres://reader:pass@localhost:5432/secafs',
    skip_schema_init=True,
))
```

### Using an Existing Connection

If you already hold a connection, open SecAFS directly on it:

```python
from secafs_sdk.db import connect_postgres

db = await connect_postgres('postgres://localhost/secafs')
agent = await SecAFS.open_with(db)
```

## Context Manager Support

Use SecAFS with async context managers:

```python
async with await SecAFS.open(SecAFSOptions(postgres_url='postgres://localhost/secafs')) as agent:
    await agent.kv.set('key', 'value')
    # The connection is automatically closed when exiting the context
```

## Development

### Setup

```bash
# Install dependencies
uv sync --group dev

# Run tests
uv run pytest

# Format code
uv run ruff format secafs_sdk tests

# Check code
uv run ruff check secafs_sdk tests
```

## License

Mulan Permissive Software License, Version 2 (MulanPSL-2.0) - see http://license.coscl.org.cn/MulanPSL2

## Links

- TypeScript SDK: see `sdk/typescript` in this repository.
