// How the "soli" debug type reaches its adapter: `soli dap` on stdio for a
// launch, the `--inspect` port of a running `soli serve --dev` for an attach.
// Pure, so it is tested without the editor.

export type AdapterChoice =
  | { kind: "executable"; command: string; args: string[] }
  | { kind: "server"; host: string; port: number }
  | { kind: "error"; message: string };

export interface DebugConfiguration {
  request?: string;
  host?: unknown;
  port?: unknown;
}

export const DEFAULT_INSPECT_PORT = 9229;

export function adapterFor(configuration: DebugConfiguration, soliPath: string | undefined): AdapterChoice {
  if (configuration.request === "attach") {
    const port = configuration.port ?? DEFAULT_INSPECT_PORT;
    if (typeof port !== "number" || !Number.isInteger(port) || port < 1 || port > 65535) {
      return { kind: "error", message: `"port" must be a port number, not ${JSON.stringify(port)}` };
    }
    const host = configuration.host ?? "127.0.0.1";
    if (typeof host !== "string" || host === "") {
      return { kind: "error", message: `"host" must be a host name or address` };
    }
    return { kind: "server", host, port };
  }
  if (!soliPath) {
    return {
      kind: "error",
      message: "No `soli` executable was found to run `soli dap`: set soli.lsp.path, or install Soli.",
    };
  }
  return { kind: "executable", command: soliPath, args: ["dap"] };
}
