import { describe, it, expect, beforeAll, beforeEach, afterEach } from "vitest";
import { SecAFS } from "../src/index_node.js";
import type { SecAFSCore } from "../src/secafs.js";
import { canConnect, setupTestDb, type TestDb } from "./helpers/db.js";

describe("SecAFS Integration Tests", () => {
  let connected = false;
  let agent: SecAFSCore;
  let testDb: TestDb;

  beforeAll(async () => {
    connected = await canConnect();
  });

  beforeEach(async (ctx) => {
    if (!connected) {
      ctx.skip();
      return;
    }
    testDb = await setupTestDb();
    agent = await SecAFS.openWith(testDb.db);
  });

  afterEach(async () => {
    if (testDb) {
      // openWith shares the helper's client, so close happens in teardown.
      await testDb.teardown();
      testDb = undefined as unknown as TestDb;
    }
  });

  describe("Initialization", () => {
    it("should successfully initialize from a database client", async () => {
      expect(agent).toBeDefined();
      expect(agent).toBeInstanceOf(SecAFS);
    });

    it("should expose the kv, fs and tools components", async () => {
      expect(agent.kv).toBeDefined();
      expect(agent.fs).toBeDefined();
      expect(agent.tools).toBeDefined();
    });

    it("should require a postgresUrl when using open()", async () => {
      // @ts-expect-error - Testing runtime validation for JS users
      await expect(SecAFS.open({})).rejects.toThrow(
        "SecAFS.open() requires 'postgresUrl'."
      );
    });

    it("should allow multiple instances over isolated databases", async () => {
      const other = await setupTestDb();
      try {
        const agent2 = await SecAFS.openWith(other.db);
        expect(agent).toBeDefined();
        expect(agent2).toBeDefined();
        expect(agent).not.toBe(agent2);
      } finally {
        await other.teardown();
      }
    });
  });

  describe("Persistence", () => {
    it("should persist data across instances over the same database", async () => {
      await agent.kv.set("test", "value1");

      // Reopen a fresh instance over the same client/schema.
      const agent2 = await SecAFS.openWith(testDb.db);
      const value = await agent2.kv.get("test");

      expect(value).toBe("value1");
    });
  });
});
