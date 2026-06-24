import { KvStore } from './kvstore.js';
import { SecAFS as Filesystem } from './filesystem/index.js';
import { ToolCalls } from './toolcalls.js';
import type { DbClient } from './db.js';

/**
 * Configuration options for opening a SecAFS instance
 */
export interface SecAFSOptions {
  /**
   * Postgres connection URL (required)
   */
  postgresUrl: string;
  /**
   * Postgres pool size (defaults to 4)
   */
  postgresPoolSize?: number;
}

export class SecAFSCore {
  private db: DbClient;

  public readonly kv: KvStore;
  public readonly fs: Filesystem;
  public readonly tools: ToolCalls;

  /**
   * Private constructor - use SecAFS.open() instead
   */
  protected constructor(db: DbClient, kv: KvStore, fs: Filesystem, tools: ToolCalls) {
    this.db = db;
    this.kv = kv;
    this.fs = fs;
    this.tools = tools;
  }

  /**
   * Get the underlying Database instance
   */
  getDatabase(): DbClient {
    return this.db;
  }

  /**
   * Close the database connection
   */
  async close(): Promise<void> {
    await this.db.close();
  }
}
