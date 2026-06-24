export type DbBackend = 'postgres' | 'opengauss';

export interface DbStatement {
  run(...params: unknown[]): Promise<unknown>;
  get<T = any>(...params: unknown[]): Promise<T | undefined>;
  all<T = any>(...params: unknown[]): Promise<T[]>;
}

export interface DbClient {
  backend: DbBackend;
  exec(sql: string): Promise<void>;
  prepare(sql: string): DbStatement;
  close(): Promise<void>;
}

export function isDbClient(value: unknown): value is DbClient {
  return (
    typeof value === 'object'
    && value !== null
    && 'backend' in value
    && 'exec' in value
    && 'prepare' in value
  );
}

export function rebindSql(sql: string, params: unknown[]): { sql: string; params: unknown[] } {
  let index = 1;
  let out = '';
  for (let i = 0; i < sql.length; i += 1) {
    const ch = sql[i];
    if (ch === '?') {
      let j = i + 1;
      while (j < sql.length && sql[j] >= '0' && sql[j] <= '9') {
        j += 1;
      }
      out += `$${index}`;
      index += 1;
      i = j - 1;
    } else {
      out += ch;
    }
  }
  return { sql: out, params };
}

// ---------------------------------------------------------------------------
// OpenGauss SQL compatibility
// ---------------------------------------------------------------------------
// OpenGauss 5.0/6.0 (PG 9.2 kernel) does NOT support
// ``ON CONFLICT ... DO UPDATE SET col = EXCLUDED.col`` (PG 9.5+).
// It supports MySQL-compatible ``ON DUPLICATE KEY UPDATE col = VALUES(col)``.

const ON_CONFLICT_DO_UPDATE_RE = /\bON\s+CONFLICT\s*(?:\([^)]*\))?\s*DO\s+UPDATE\s+SET\s+/i;
const ON_CONFLICT_DO_NOTHING_RE = /\bON\s+CONFLICT\s*(?:\([^)]*\))?\s*DO\s+NOTHING\b/i;
const EXCLUDED_REF_RE = /\bEXCLUDED\.(\w+)/gi;

/**
 * Rewrite PostgreSQL ``ON CONFLICT`` syntax to OpenGauss-compatible syntax.
 *
 * - ``ON CONFLICT (...) DO UPDATE SET col = EXCLUDED.col``
 *   → ``ON DUPLICATE KEY UPDATE col = VALUES(col)``
 * - ``ON CONFLICT (...) DO NOTHING``
 *   → removed entirely
 */
export function rewriteOnConflictForOpenGauss(sql: string): string {
  // Handle DO NOTHING first (strip the clause)
  sql = sql.replace(ON_CONFLICT_DO_NOTHING_RE, '');
  // Handle DO UPDATE SET
  if (ON_CONFLICT_DO_UPDATE_RE.test(sql)) {
    sql = sql.replace(ON_CONFLICT_DO_UPDATE_RE, 'ON DUPLICATE KEY UPDATE ');
    sql = sql.replace(EXCLUDED_REF_RE, 'VALUES($1)');
  }
  return sql;
}
