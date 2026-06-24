#!/bin/sh
set -e

echo -n "TEST init... "

TEST_AGENT_ID="test-agent"

# Cleanup any existing test database (not the entire .secafs directory!)
rm -f ".secafs/${TEST_AGENT_ID}.db" ".secafs/${TEST_AGENT_ID}.db-shm" ".secafs/${TEST_AGENT_ID}.db-wal"

# Test: Run init command with specific ID
if ! output=$(cargo run -- init "$TEST_AGENT_ID" 2>&1); then
    echo "FAILED: init command failed"
    echo "Output was: $output"
    exit 1
fi

# Check that .secafs directory was created
if [ ! -d .secafs ]; then
    echo "FAILED: .secafs directory was not created"
    echo "Output was: $output"
    exit 1
fi

# Check that the database file was created in .secafs
if [ ! -f ".secafs/$TEST_AGENT_ID.db" ]; then
    echo "FAILED: secafs database was not created in .secafs directory"
    echo "Output was: $output"
    exit 1
fi

# Check that output contains success message with .secafs path
echo "$output" | grep -q "Created agent filesystem: .secafs/$TEST_AGENT_ID.db" || {
    echo "FAILED: Expected success message not found in output"
    echo "Output was: $output"
    rm -f ".secafs/${TEST_AGENT_ID}.db" ".secafs/${TEST_AGENT_ID}.db-shm" ".secafs/${TEST_AGENT_ID}.db-wal"
    exit 1
}

# Test: Running init again should fail without --force
if cargo run -- init "$TEST_AGENT_ID" 2>&1 | grep -q "already exists"; then
    : # Expected behavior
else
    echo "FAILED: init should fail when secafs database already exists"
    rm -f ".secafs/${TEST_AGENT_ID}.db" ".secafs/${TEST_AGENT_ID}.db-shm" ".secafs/${TEST_AGENT_ID}.db-wal"
    exit 1
fi

# Test: Running init with --force should succeed
if ! output=$(cargo run -- init "$TEST_AGENT_ID" --force 2>&1); then
    echo "FAILED: init --force command failed"
    echo "Output was: $output"
    rm -f ".secafs/${TEST_AGENT_ID}.db" ".secafs/${TEST_AGENT_ID}.db-shm" ".secafs/${TEST_AGENT_ID}.db-wal"
    exit 1
fi

# Check that output contains success message
echo "$output" | grep -q "Created agent filesystem: .secafs/$TEST_AGENT_ID.db" || {
    echo "FAILED: Expected success message not found in init --force output"
    echo "Output was: $output"
    rm -f ".secafs/${TEST_AGENT_ID}.db" ".secafs/${TEST_AGENT_ID}.db-shm" ".secafs/${TEST_AGENT_ID}.db-wal"
    exit 1
}

# Cleanup test database only
rm -f ".secafs/${TEST_AGENT_ID}.db" ".secafs/${TEST_AGENT_ID}.db-shm" ".secafs/${TEST_AGENT_ID}.db-wal"

echo "OK"
