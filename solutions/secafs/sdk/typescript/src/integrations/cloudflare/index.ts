/**
 * Cloudflare Durable Objects integration for SecAFS.
 *
 * Provides SecAFS - a FileSystem implementation that uses
 * Cloudflare Durable Objects SQLite storage as its backend.
 *
 * @example
 * ```typescript
 * import { SecAFS } from "secafs-sdk/cloudflare";
 *
 * export class MyDurableObject extends DurableObject {
 *   private fs: SecAFS;
 *
 *   constructor(ctx: DurableObjectState, env: Env) {
 *     super(ctx, env);
 *     this.fs = SecAFS.create(ctx.storage);
 *   }
 *
 *   async fetch(request: Request) {
 *     await this.fs.writeFile('/hello.txt', 'Hello, World!');
 *     const content = await this.fs.readFile('/hello.txt', 'utf8');
 *     return new Response(content);
 *   }
 * }
 * ```
 *
 * @see https://developers.cloudflare.com/durable-objects/
 */

export { SecAFS, type CloudflareStorage } from "./secafs.js";

export type {
  FileSystem,
  Stats,
  DirEntry,
  FilesystemStats,
  FileHandle,
} from "../../filesystem/interface.js";
