import { SecAFSCore } from "./secafs.js";
import { KvStore } from "./kvstore.js";
import { SecAFS as SecAFSImpl } from "./filesystem/index.js";
import { ToolCalls } from "./toolcalls.js";
import { Buffer } from "buffer";
import { isDbClient, type DbClient } from "./db.js";

export class SecAFS extends SecAFSCore {
    static async openWith(db: DbClient): Promise<SecAFSCore> {
        const client = isDbClient(db) ? db : db;
        const [kv, fs, tools] = await Promise.all([
            KvStore.fromDatabase(client),
            SecAFSImpl.fromDatabase(client, Buffer),
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
