/**
 * `openclaw secafs ...` CLI, folded INTO the plugin (Cat 3).
 *
 * Previously these subcommands lived in OpenClaw core (`src/cli/secafs-cli.ts`
 * + `src/commands/secafs-cli-runner.ts`). In the componentized form they are
 * registered by the external plugin via `api.registerCli`, so the OpenClaw
 * checkout stays free of secafs-specific code.
 *
 * Each subcommand is a thin gateway client: it calls the plugin's own
 * `secafs.*` gateway methods on the running gateway using the public
 * `callGatewayFromCli` helper from `openclaw/plugin-sdk/gateway-runtime`
 * (core's internal `gateway/call.js` is NOT a public SDK export). Routing to
 * the gateway means the command hits the in-process plugin handlers, which own
 * the full session-lifecycle context (mount, workspace redirect, etc.).
 */
import { callGatewayFromCli } from "openclaw/plugin-sdk/gateway-runtime";

/**
 * Minimal structural view of the commander `Command` surface this file uses,
 * so the plugin does not take a hard dependency on commander's types.
 */
interface CliCommand {
  command(nameAndArgs: string): CliCommand;
  description(text: string): CliCommand;
  option(flags: string, description?: string, defaultValue?: unknown): CliCommand;
  action(fn: (...args: unknown[]) => unknown): CliCommand;
}

/** Context handed to a plugin CLI registrar (`OpenClawPluginCliContext`). */
interface CliContext {
  program: CliCommand;
}

function withGatewayOptions(cmd: CliCommand): CliCommand {
  return cmd
    .option("--url <url>", "Gateway WebSocket URL (defaults to configured gateway)")
    .option("--token <token>", "Gateway token (if required)")
    .option("--timeout <ms>", "Timeout in ms", "10000");
}

async function run(method: string, opts: unknown, params?: unknown): Promise<void> {
  const result = await callGatewayFromCli(method, opts as never, params);
  process.stdout.write(`${JSON.stringify(result, null, 2)}\n`);
}

export function registerSecafsCli(ctx: CliContext): void {
  const secafs = ctx.program.command("secafs").description("SecAFS Chat session helpers");

  withGatewayOptions(secafs.command("status"))
    .description("Show SecAFS daemon status")
    .action((opts) => run("secafs.status", opts));

  withGatewayOptions(secafs.command("create"))
    .description("Create a new SecAFS chat session (mounts on success)")
    .action((opts) => run("secafs.session.create", opts));

  withGatewayOptions(secafs.command("open <sessionKey>"))
    .description("Mount an existing SecAFS session")
    .action((sessionKey, opts) => run("secafs.session.open", opts, { sessionKey }));

  withGatewayOptions(secafs.command("close <sessionKey>"))
    .description("Unmount a SecAFS session without deleting its data")
    .action((sessionKey, opts) => run("secafs.session.close", opts, { sessionKey }));

  withGatewayOptions(secafs.command("destroy <sessionKey>"))
    .description("Unmount and permanently delete a SecAFS session")
    .action((sessionKey, opts) => run("secafs.session.destroy", opts, { sessionKey }));
}
