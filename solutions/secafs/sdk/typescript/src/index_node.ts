import { SecAFSCore, SecAFSOptions } from "./secafs.js";
import { KvStore } from "./kvstore.js";
import { SecAFS as Filesystem } from "./filesystem/index.js";
import { ToolCalls } from "./toolcalls.js";
import { createPostgresDb } from "./db_node.js";
import { isDbClient, type DbClient } from "./db.js";

export class SecAFS extends SecAFSCore {
    /**
   * Open a SecAFS filesystem (PostgreSQL backend)
   * @param options Configuration options with postgresUrl
   * @returns Fully initialized SecAFS instance
   * @example
   * ```typescript
   * const agent = await SecAFS.open({ postgresUrl: 'postgres://user:pass@host/db' });
   * ```
   */
    static async open(options: SecAFSOptions): Promise<SecAFS> {
        const { postgresUrl, postgresPoolSize } = options;

        if (!postgresUrl) {
            throw new Error("SecAFS.open() requires 'postgresUrl'.");
        }

        const db = await createPostgresDb(postgresUrl, postgresPoolSize ?? 4);
        return await this.openWith(db);
    }

    static async openWith(db: DbClient): Promise<SecAFSCore> {
        const client = isDbClient(db) ? db : db;
        const [kv, fs, tools] = await Promise.all([
            KvStore.fromDatabase(client),
            Filesystem.fromDatabase(client),
            ToolCalls.fromDatabase(client),
        ]);
        return new SecAFS(client, kv, fs, tools);
    }
}

export { SecAFSOptions } from './secafs.js';
export { KvStore } from './kvstore.js';
export { SecAFS as Filesystem } from './filesystem/index.js';
export type { Stats, DirEntry, FilesystemStats, FileHandle, FileSystem } from './filesystem/index.js';
export { ToolCalls } from './toolcalls.js';
export type { ToolCall, ToolCallStats } from './toolcalls.js';
