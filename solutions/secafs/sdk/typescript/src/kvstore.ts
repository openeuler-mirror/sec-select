import type { DbClient } from './db.js';

export class KvStore {
  private db: DbClient;

  private constructor(db: DbClient) {
    this.db = db;
  }

  /**
   * Create a KvStore from an existing database connection
   */
  static async fromDatabase(db: DbClient): Promise<KvStore> {
    const kv = new KvStore(db);
    await kv.initialize();
    return kv;
  }

  private async initialize(): Promise<void> {
    await this.db.exec(`
      CREATE TABLE IF NOT EXISTS kv_store (
        key TEXT PRIMARY KEY,
        value TEXT NOT NULL,
        created_at BIGINT DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT),
        updated_at BIGINT DEFAULT (EXTRACT(EPOCH FROM NOW())::BIGINT)
      )
    `);

    // Create index on created_at for potential queries
    await this.db.exec(`
      CREATE INDEX IF NOT EXISTS idx_kv_store_created_at
      ON kv_store(created_at)
    `);
  }

  async set(key: string, value: any): Promise<void> {
    // Serialize the value to JSON
    const serializedValue = JSON.stringify(value);

    const stmt = this.db.prepare(`
      INSERT INTO kv_store (key, value, updated_at)
      VALUES (?, ?, EXTRACT(EPOCH FROM NOW())::BIGINT)
      ON CONFLICT(key) DO UPDATE SET
        value = excluded.value,
        updated_at = EXTRACT(EPOCH FROM NOW())::BIGINT
    `);

    await stmt.run(key, serializedValue);
  }

  async get<T = any>(key: string): Promise<T | undefined> {
    const stmt = this.db.prepare(`SELECT value FROM kv_store WHERE key = ?`);
    const row = await stmt.get(key) as { value: string } | undefined;

    if (!row) {
      return undefined;
    }

    // Deserialize the JSON value
    return JSON.parse(row.value) as T;
  }

  async list(prefix: string): Promise<{ key: string, value: any }[]> {
    const stmt = this.db.prepare(`SELECT key, value FROM kv_store WHERE key LIKE ? ESCAPE '\\'`);
    const escaped = prefix.replace('\\', '\\\\').replace('%', '\\%').replace('_', '\\_');
    const rows = await stmt.all(escaped + '%') as { key: string, value: string }[];
    return rows.map(r => ({ key: r.key, value: JSON.parse(r.value) }));
  }

  async delete(key: string): Promise<void> {
    const stmt = this.db.prepare(`DELETE FROM kv_store WHERE key = ?`);
    await stmt.run(key);
  }
}
