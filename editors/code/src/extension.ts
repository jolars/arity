import * as vscode from "vscode";
import * as fs from "node:fs/promises";
import * as path from "node:path";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  Trace,
} from "vscode-languageclient/node";
import { resolveArityBinary } from "./installer";

let client: LanguageClient | undefined;

async function isNixOs(): Promise<boolean> {
  if (process.platform !== "linux") {
    return false;
  }
  try {
    const osRelease = await fs.readFile("/etc/os-release", "utf8");
    return /(^|\n)ID=nixos(\n|$)/.test(osRelease);
  } catch {
    return false;
  }
}

function isReleaseTagExplicitlyConfigured(
  config: vscode.WorkspaceConfiguration,
): boolean {
  const value = config.inspect<string>("releaseTag");
  return (
    value?.globalValue !== undefined ||
    value?.workspaceValue !== undefined ||
    value?.workspaceFolderValue !== undefined
  );
}

type ExecutableStrategy = "bundled" | "environment" | "path";

async function findBundledBinary(
  context: vscode.ExtensionContext,
): Promise<string | undefined> {
  const binaryName = process.platform === "win32" ? "arity.exe" : "arity";
  const candidate = path.join(context.extensionPath, "server", binaryName);
  try {
    await fs.access(candidate);
    return candidate;
  } catch {
    return undefined;
  }
}

async function resolveCommandPath(
  context: vscode.ExtensionContext,
  config: vscode.WorkspaceConfiguration,
  outputChannel: vscode.LogOutputChannel,
): Promise<string> {
  const githubRepo = config.get<string>("githubRepo", "jolars/arity");
  const version = config.get<string>("version", "latest");
  const releaseTag = config.get<string>("releaseTag", "latest");
  const releaseTagExplicit = isReleaseTagExplicitlyConfigured(config);
  const selectedRelease = releaseTagExplicit ? releaseTag : version;
  const versionInspect = config.inspect<string>("version");
  const versionPinExplicit =
    releaseTagExplicit ||
    versionInspect?.globalValue !== undefined ||
    versionInspect?.workspaceValue !== undefined ||
    versionInspect?.workspaceFolderValue !== undefined;

  const strategy = config.get<ExecutableStrategy>(
    "executableStrategy",
    "bundled",
  );

  if (strategy === "path") {
    const executablePath = config.get<string | null>("executablePath", null);
    if (!executablePath || executablePath.trim().length === 0) {
      void vscode.window.showWarningMessage(
        "arity.executableStrategy is set to 'path' but arity.executablePath is empty. Falling back to 'arity' on PATH.",
      );
      return "arity";
    }
    return executablePath;
  }

  if (strategy === "environment") {
    return "arity";
  }

  // strategy === "bundled": prefer the bundled binary, then download from
  // GitHub releases, and finally fall back to PATH.
  if (!versionPinExplicit) {
    const bundled = await findBundledBinary(context);
    if (bundled) {
      outputChannel.appendLine(`Using bundled Arity binary at ${bundled}.`);
      return bundled;
    }
  }

  const nixOs = await isNixOs();
  if (nixOs) {
    outputChannel.appendLine(
      "Detected NixOS; skipping binary download and using 'arity' on PATH.",
    );
    return "arity";
  }

  try {
    return await resolveArityBinary(
      context.globalStorageUri.fsPath,
      githubRepo,
      selectedRelease,
    );
  } catch (error) {
    const message =
      error instanceof Error ? error.message : "Unknown download error";
    void vscode.window.showWarningMessage(
      `Arity binary download failed (${message}). Falling back to 'arity' on PATH.`,
    );
    return "arity";
  }
}

function mergeServerEnvironment(
  baseEnv: NodeJS.ProcessEnv,
  overrides: Record<string, string>,
  extraPathEntries: string[],
): NodeJS.ProcessEnv {
  const env: NodeJS.ProcessEnv = { ...baseEnv, ...overrides };
  const normalizedExtraPath = extraPathEntries
    .map((entry) => entry.trim())
    .filter((entry) => entry.length > 0);

  if (normalizedExtraPath.length === 0) {
    return env;
  }

  const pathKey =
    process.platform === "win32"
      ? Object.keys(env).find((key) => key.toLowerCase() === "path") ?? "Path"
      : "PATH";

  for (const key of Object.keys(env)) {
    if (key !== pathKey && key.toLowerCase() === "path") {
      delete env[key];
    }
  }

  const existingPath = env[pathKey]?.trim() ?? "";
  env[pathKey] =
    normalizedExtraPath.join(path.delimiter) +
    (existingPath ? `${path.delimiter}${existingPath}` : "");

  return env;
}

async function startClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.LogOutputChannel,
): Promise<void> {
  const config = vscode.workspace.getConfiguration("arity");
  const commandPath = await resolveCommandPath(context, config, outputChannel);

  const serverArgs = config.get<string[]>("serverArgs", []);
  const userServerEnv = config.get<Record<string, string>>("serverEnv", {});
  const logLevel = config.get<string | null>("logLevel", null);
  const serverEnv: Record<string, string> = {
    ...(logLevel ? { RUST_LOG: logLevel } : {}),
    ...userServerEnv,
  };
  const extraPath = config.get<string[]>("extraPath", []);
  const traceLevel = config.get<"off" | "messages" | "verbose">(
    "trace.server",
    "off",
  );

  const serverOptions: ServerOptions = {
    command: commandPath,
    args: ["lsp", ...serverArgs],
    options: {
      env: mergeServerEnvironment(process.env, serverEnv, extraPath),
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "r" },
      { scheme: "untitled", language: "r" },
      { scheme: "file", pattern: "**/*.R" },
      { scheme: "file", pattern: "**/*.r" },
    ],
    outputChannel,
    traceOutputChannel: outputChannel,
  };

  client = new LanguageClient(
    "arityLanguageServer",
    "Arity Language Server",
    serverOptions,
    clientOptions,
  );

  try {
    await client.start();
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    outputChannel.appendLine(
      `Failed to start Arity language server: ${message}`,
    );
    void vscode.window.showErrorMessage(
      `Arity language server failed to start: ${message}`,
    );
    client = undefined;
    return;
  }
  if (traceLevel === "messages") {
    void client.setTrace(Trace.Messages);
  } else if (traceLevel === "verbose") {
    void client.setTrace(Trace.Verbose);
  }
}

async function restartClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.LogOutputChannel,
): Promise<void> {
  if (client) {
    try {
      await client.stop();
    } catch (error) {
      const message = error instanceof Error ? error.message : String(error);
      outputChannel.appendLine(
        `Error stopping Arity language server: ${message}`,
      );
    }
    client = undefined;
  }
  await startClient(context, outputChannel);
}

export async function activate(
  context: vscode.ExtensionContext,
): Promise<void> {
  const outputChannel = vscode.window.createOutputChannel(
    "Arity Language Server",
    { log: true },
  );
  context.subscriptions.push(outputChannel);

  context.subscriptions.push(
    vscode.commands.registerCommand("arity.restart", () =>
      restartClient(context, outputChannel),
    ),
  );

  await startClient(context, outputChannel);
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
  }
}
