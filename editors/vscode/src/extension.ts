// Soli for VS Code, Cursor, VSCodium and Windsurf: starts `soli lsp` and,
// only when the language server cannot run, lints with `soli lint`.

import { spawn } from "child_process";
import * as os from "os";
import * as vscode from "vscode";
import {
  CloseAction,
  ErrorAction,
  LanguageClient,
  LanguageClientOptions,
  RevealOutputChannelOn,
  ServerOptions,
  State,
  Trace,
} from "vscode-languageclient/node";
import { fallbackLintActive, lacksLanguageServer } from "./lint";
import { LintFallback } from "./lintFallback";
import { defaultIsExecutable, Resolution, resolveSoli } from "./resolveBinary";

const INSTALL_DOCS = "https://soli.solisoft.net/docs/getting-started/installation";
/** More crashes than this within the window and the server stays down. */
const MAX_RESTARTS = 4;
const RESTART_WINDOW_MS = 3 * 60 * 1000;

let output: vscode.OutputChannel;
let fallback: LintFallback;
let client: LanguageClient | undefined;
/** The resolved `soli`, also used by the fallback lint. */
let soliPath: string | undefined;
/** Set while the extension itself stops the client, so the stop is not
 * mistaken for a crash. */
let stopping = false;

export async function activate(context: vscode.ExtensionContext): Promise<void> {
  output = vscode.window.createOutputChannel("Soli");
  fallback = new LintFallback(() => soliPath);
  context.subscriptions.push(output, fallback);

  context.subscriptions.push(
    vscode.commands.registerCommand("soli.restartServer", async () => {
      await stopServer();
      await start();
    }),
    vscode.commands.registerCommand("soli.showOutput", () => output.show(true)),
    vscode.commands.registerCommand("soli.lint", () => {
      const document = vscode.window.activeTextEditor?.document;
      if (!document) {
        return;
      }
      if (serverActive()) {
        vscode.window.showInformationMessage(
          "The Soli language server is running and already reports the lint findings for this file.",
        );
        return;
      }
      fallback.lint(document);
    }),
    vscode.workspace.onDidChangeConfiguration(async (event) => {
      if (event.affectsConfiguration("soli.lsp") || event.affectsConfiguration("soli.lint.executablePath")) {
        await stopServer();
        await start();
      } else if (event.affectsConfiguration("soli.lint")) {
        refreshFallback();
      }
    }),
  );

  await start();
}

export async function deactivate(): Promise<void> {
  await stopServer();
}

function settings(): vscode.WorkspaceConfiguration {
  return vscode.workspace.getConfiguration("soli");
}

function resolve(): Resolution {
  const config = settings();
  // `soli.lint.executablePath` is the 0.2 setting, still honoured when the
  // new one is empty. Its old default, "soli", resolves on PATH as before.
  const configured = config.get<string>("lsp.path") || config.get<string>("lint.executablePath") || "";
  return resolveSoli(configured, {
    platform: process.platform,
    env: process.env,
    homedir: os.homedir(),
    isExecutable: defaultIsExecutable,
  });
}

function serverActive(): boolean {
  return client !== undefined && client.state !== State.Stopped;
}

/** Turn the `soli lint` fallback on or off to match the server. */
function refreshFallback(): void {
  const active = fallbackLintActive({
    lintEnabled: settings().get<boolean>("lint.enable", true) && soliPath !== undefined,
    serverActive: serverActive(),
  });
  if (active && !fallback.isActive) {
    output.appendLine("Linting with `soli lint` on open and save (the language server is not running).");
    fallback.enable();
  } else if (!active && fallback.isActive) {
    fallback.disable();
  }
}

async function start(): Promise<void> {
  const resolution = resolve();
  soliPath = resolution.path;
  if (!soliPath) {
    output.appendLine(`soli not found. Looked in:\n  ${resolution.tried.join("\n  ")}`);
    showNotFound(resolution.tried);
    refreshFallback();
    return;
  }
  if (!settings().get<boolean>("lsp.enabled", true)) {
    output.appendLine("The language server is disabled (soli.lsp.enabled).");
    refreshFallback();
    return;
  }
  await startServer(soliPath);
  refreshFallback();
}

async function showNotFound(tried: string[]): Promise<void> {
  const choice = await vscode.window.showErrorMessage(
    `Could not find the \`soli\` executable (looked in ${tried.length} places: ${tried.slice(0, 4).join(", ")}${tried.length > 4 ? ", …" : ""}). Install Soli, or set soli.lsp.path to its absolute path.`,
    "Set Path",
    "Install Soli",
    "Show Output",
  );
  if (choice === "Set Path") {
    await vscode.commands.executeCommand("workbench.action.openSettings", "soli.lsp.path");
  } else if (choice === "Install Soli") {
    await vscode.env.openExternal(vscode.Uri.parse(INSTALL_DOCS));
  } else if (choice === "Show Output") {
    output.show(true);
  }
}

async function startServer(executable: string): Promise<void> {
  let stderr = "";
  let lacksServer = false;
  const restarts: number[] = [];

  const serverOptions: ServerOptions = async () => {
    stderr = "";
    const folder = vscode.workspace.workspaceFolders?.[0]?.uri;
    const child = spawn(executable, ["lsp"], {
      cwd: folder?.scheme === "file" ? folder.fsPath : undefined,
      env: process.env,
    });
    child.stderr?.on("data", (chunk: Buffer) => {
      stderr = (stderr + chunk.toString()).slice(-4096);
    });
    child.on("exit", (code) => {
      if (lacksLanguageServer(code, stderr)) {
        lacksServer = true;
      }
    });
    return child;
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: "file", language: "soli" },
      { scheme: "file", language: "soli-template" },
      { scheme: "untitled", language: "soli" },
    ],
    outputChannel: output,
    revealOutputChannelOn: RevealOutputChannelOn.Never,
    errorHandler: {
      error: (_error, _message, count) => ({
        action: (count ?? 0) <= 3 ? ErrorAction.Continue : ErrorAction.Shutdown,
        handled: true,
      }),
      closed: () => {
        if (lacksServer || lacksLanguageServer(null, stderr)) {
          lacksServer = true;
          return { action: CloseAction.DoNotRestart, handled: true };
        }
        const now = Date.now();
        restarts.push(now);
        while (restarts.length > 0 && now - restarts[0] > RESTART_WINDOW_MS) {
          restarts.shift();
        }
        if (restarts.length > MAX_RESTARTS) {
          return {
            action: CloseAction.DoNotRestart,
            message:
              "The Soli language server stopped 5 times in 3 minutes and will not be restarted; `soli lint` runs on save instead. See the Soli output, then run “Soli: Restart Language Server”.",
          };
        }
        return { action: CloseAction.Restart, handled: true };
      },
    },
  };

  const languageClient = new LanguageClient("soli", "Soli Language Server", serverOptions, clientOptions);
  client = languageClient;
  languageClient.onDidChangeState((event) => {
    if (event.newState === State.Stopped && !stopping && client === languageClient) {
      if (lacksServer) {
        reportNoServer(executable);
      }
      refreshFallback();
    }
  });

  output.appendLine(`Starting ${executable} lsp`);
  try {
    await languageClient.start();
  } catch (error) {
    output.appendLine(`The language server did not start: ${String(error)}`);
    if (lacksServer || lacksLanguageServer(null, stderr)) {
      reportNoServer(executable);
    } else {
      vscode.window.showWarningMessage(
        `The Soli language server did not start (${executable} lsp); \`soli lint\` runs on save instead.`,
        "Show Output",
      ).then((choice) => choice && output.show(true));
    }
    if (client === languageClient) {
      client = undefined;
    }
    return;
  }

  const info = languageClient.initializeResult?.serverInfo;
  output.appendLine(`Connected to ${info?.name ?? "soli"} ${info?.version ?? "(version unknown)"} at ${executable}`);
  const trace = settings().get<string>("lsp.trace", "off");
  await languageClient.setTrace(Trace.fromString(trace));
}

function reportNoServer(executable: string): void {
  output.appendLine(`${executable} has no language server (built without the \`lsp\` feature).`);
  vscode.window.showWarningMessage(
    `This soli (${executable}) was built without the language server, so only \`soli lint\` runs. Install a release build of Soli, or set soli.lsp.path to one.`,
    "Install Soli",
  ).then((choice) => {
    if (choice === "Install Soli") {
      vscode.env.openExternal(vscode.Uri.parse(INSTALL_DOCS));
    }
  });
}

async function stopServer(): Promise<void> {
  const running = client;
  client = undefined;
  if (!running) {
    return;
  }
  stopping = true;
  try {
    if (running.state !== State.Stopped) {
      await running.stop();
    }
  } catch (error) {
    output.appendLine(`Stopping the language server: ${String(error)}`);
  } finally {
    stopping = false;
  }
}
