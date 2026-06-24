import { Pool } from 'pg';

import type { DbBackend, DbClient, DbStatement } from './db.js';
import { rebindSql, rewriteOnConflictForOpenGauss } from './db.js';

class PgStatement implements DbStatement {
  private pool: Pool;
  private sql: string;
  private backend: DbBackend;

  constructor(pool: Pool, sql: string, backend: DbBackend) {
    this.pool = pool;
    this.sql = sql;
    this.backend = backend;
  }

  async run(...params: unknown[]): Promise<unknown> {
    let { sql, params: bound } = rebindSql(this.sql, params);
    if (this.backend === 'opengauss') sql = rewriteOnConflictForOpenGauss(sql);
    return this.pool.query(sql, bound);
  }

  async get<T = any>(...params: unknown[]): Promise<T | undefined> {
    let { sql, params: bound } = rebindSql(this.sql, params);
    if (this.backend === 'opengauss') sql = rewriteOnConflictForOpenGauss(sql);
    const result = await this.pool.query(sql, bound);
    return result.rows[0] as T | undefined;
  }

  async all<T = any>(...params: unknown[]): Promise<T[]> {
    let { sql, params: bound } = rebindSql(this.sql, params);
    if (this.backend === 'opengauss') sql = rewriteOnConflictForOpenGauss(sql);
    const result = await this.pool.query(sql, bound);
    return result.rows as T[];
  }
}

export class PgDb implements DbClient {
  public readonly backend: DbBackend;
  private pool: Pool;

  constructor(pool: Pool, backend: DbBackend = 'postgres') {
    this.pool = pool;
    this.backend = backend;
  }

  async exec(sql: string): Promise<void> {
    let { sql: boundSql } = rebindSql(sql, []);
    if (this.backend === 'opengauss') boundSql = rewriteOnConflictForOpenGauss(boundSql);
    await this.pool.query(boundSql);
  }

  prepare(sql: string): DbStatement {
    return new PgStatement(this.pool, sql, this.backend);
  }

  async close(): Promise<void> {
    await this.pool.end();
  }
}

/**
 * Normalize an `opengauss://` URL to `postgres://` so that the `pg` driver
 * can connect to OpenGauss databases using the PostgreSQL wire protocol.
 */
function normalizeDbUrl(url: string): string {
  if (url.startsWith('opengauss://')) {
    return url.replace('opengauss://', 'postgres://');
  }
  return url;
}

/**
 * Detect the database backend from the URL scheme.
 * `opengauss://` → `'opengauss'`, otherwise `'postgres'`.
 */
function detectBackend(url: string): DbBackend {
  return url.startsWith('opengauss://') ? 'opengauss' : 'postgres';
}

export async function createPostgresDb(url: string, poolSize: number): Promise<PgDb> {
  const backend = detectBackend(url);
  const normalizedUrl = normalizeDbUrl(url);
  const pool = new Pool({
    connectionString: normalizedUrl,
    max: Math.max(1, poolSize || 4),
  });
  return new PgDb(pool, backend);
}
