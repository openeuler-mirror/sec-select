import { createPostgresDb, PgDb } from "../../src/db_node.js";
import type { DbClient } from "../../src/db.js";

const DEFAULT_URL = "opengauss://secafs:Secafs!123@localhost:5433/secafs";

export interface TestDb {
  db: DbClient;
  schema: string;
  teardown: () => Promise<void>;
}

function testUrl(): string {
  return process.env.SECAFS_TEST_POSTGRES_URL ?? DEFAULT_URL;
}

function uniqueSchema(): string {
  const suffix = Math.random().toString(36).slice(2) + Date.now().toString(36);
  return `secafs_test_${suffix}`;
}

/**
 * Connect to the configured PostgreSQL/openGauss test database and create an
 * isolated schema for a single test. The schema is set as the connection's
 * search_path, so the SDK's `CREATE TABLE IF NOT EXISTS` statements (which use
 * unqualified names) land in it. The returned teardown drops the schema and
 * closes the client.
 *
 * On any connection/setup failure the rejection propagates so the caller can
 * skip the suite cleanly.
 */
export async function setupTestDb(): Promise<TestDb> {
  const url = testUrl();
  const schema = uniqueSchema();
  // Pool size 1: a single backing connection keeps the per-test `search_path`
  // (set below) consistent across every query, including concurrent ones.
  const db = await createPostgresDb(url, 1);
  try {
    await db.exec(`CREATE SCHEMA "${schema}"`);
    await db.exec(`SET search_path TO "${schema}"`);
  } catch (err) {
    await db.close().catch(() => {});
    throw err;
  }

  return {
    db,
    schema,
    teardown: async () => {
      try {
        await db.exec(`DROP SCHEMA IF EXISTS "${schema}" CASCADE`);
      } finally {
        await db.close();
      }
    },
  };
}

/**
 * Probe whether the test database is reachable. Used by `beforeAll` guards so a
 * missing dev database skips the suite instead of failing it.
 */
export async function canConnect(): Promise<boolean> {
  let probe: PgDb | undefined;
  try {
    probe = await createPostgresDb(testUrl(), 1);
    await probe.exec("SELECT 1");
    return true;
  } catch {
    return false;
  } finally {
    if (probe) await probe.close().catch(() => {});
  }
}
