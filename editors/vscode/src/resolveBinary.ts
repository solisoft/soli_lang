// Finding the `soli` executable. Kept free of the `vscode` module so the
// unit tests run under plain Node.

import * as fs from "fs";
import * as path from "path";

export interface ResolveEnv {
  platform: NodeJS.Platform;
  env: Record<string, string | undefined>;
  homedir: string;
  /** Whether `file` exists and can be run. */
  isExecutable(file: string): boolean;
}

export interface Resolution {
  /** The executable to run, or undefined when none was found. */
  path?: string;
  /** Every location looked at, in order, for the "not found" message. */
  tried: string[];
}

export function defaultIsExecutable(file: string): boolean {
  try {
    const stat = fs.statSync(file);
    if (!stat.isFile()) {
      return false;
    }
    if (process.platform === "win32") {
      return true;
    }
    fs.accessSync(file, fs.constants.X_OK);
    return true;
  } catch {
    return false;
  }
}

function pathApi(platform: NodeJS.Platform): typeof path.posix {
  return platform === "win32" ? path.win32 : path.posix;
}

/** The binary's file names: `soli`, or `soli.exe` (and PATHEXT) on Windows. */
function executableNames(name: string, resolveEnv: ResolveEnv): string[] {
  if (resolveEnv.platform !== "win32" || path.win32.extname(name) !== "") {
    return [name];
  }
  const pathext = (resolveEnv.env.PATHEXT || ".EXE;.CMD;.BAT;.COM")
    .split(";")
    .filter((ext) => ext.length > 0)
    .map((ext) => ext.toLowerCase());
  // `.exe` first: release builds ship `soli.exe`.
  const ordered = [".exe", ...pathext.filter((ext) => ext !== ".exe")];
  return ordered.map((ext) => name + ext);
}

/** The directories `install.sh`, `cargo install` and a manual install use,
 * which an editor started from the dock or the start menu often lacks on
 * its PATH. */
export function knownInstallDirs(resolveEnv: ResolveEnv): string[] {
  const p = pathApi(resolveEnv.platform);
  // `os.homedir()`: %USERPROFILE% on Windows, $HOME elsewhere.
  const home = resolveEnv.homedir;
  const cargoHome = resolveEnv.env.CARGO_HOME;
  const dirs: string[] = [];
  if (resolveEnv.platform === "win32") {
    dirs.push(p.join(cargoHome || p.join(home, ".cargo"), "bin"));
    dirs.push(p.join(home, ".local", "bin"));
    if (resolveEnv.env.LOCALAPPDATA) {
      dirs.push(p.join(resolveEnv.env.LOCALAPPDATA, "Programs", "soli"));
    }
  } else {
    dirs.push(p.join(home, ".local", "bin"));
    dirs.push(p.join(cargoHome || p.join(home, ".cargo"), "bin"));
    dirs.push("/usr/local/bin");
    if (resolveEnv.platform === "darwin") {
      dirs.push("/opt/homebrew/bin");
    }
  }
  return dirs;
}

function expandHome(file: string, resolveEnv: ResolveEnv): string {
  if (file === "~") {
    return resolveEnv.homedir;
  }
  if (file.startsWith("~/") || file.startsWith("~\\")) {
    return pathApi(resolveEnv.platform).join(resolveEnv.homedir, file.slice(2));
  }
  return file;
}

function searchDirs(dirs: string[], name: string, resolveEnv: ResolveEnv, tried: string[]): string | undefined {
  const p = pathApi(resolveEnv.platform);
  for (const dir of dirs) {
    for (const file of executableNames(name, resolveEnv)) {
      const candidate = p.join(dir, file);
      if (tried.includes(candidate)) {
        continue;
      }
      tried.push(candidate);
      if (resolveEnv.isExecutable(candidate)) {
        return candidate;
      }
    }
  }
  return undefined;
}

/**
 * Find `soli`: the configured path first (an absolute path, `~/…`, or a bare
 * name looked up on PATH), else `soli` on PATH, else the known install
 * directories. A configured path that does not exist is not silently
 * replaced by another binary: it is reported as not found.
 */
export function resolveSoli(configured: string | undefined, resolveEnv: ResolveEnv): Resolution {
  const p = pathApi(resolveEnv.platform);
  const tried: string[] = [];
  const pathDirs = (resolveEnv.env.PATH || resolveEnv.env.Path || "")
    .split(p.delimiter)
    .filter((dir) => dir.length > 0);

  const setting = (configured || "").trim();
  if (setting.length > 0) {
    const expanded = expandHome(setting, resolveEnv);
    const hasSeparator = expanded.includes("/") || (resolveEnv.platform === "win32" && expanded.includes("\\"));
    if (!hasSeparator) {
      return { path: searchDirs(pathDirs, expanded, resolveEnv, tried), tried };
    }
    for (const file of executableNames(expanded, resolveEnv)) {
      tried.push(file);
      if (resolveEnv.isExecutable(file)) {
        return { path: file, tried };
      }
    }
    return { tried };
  }

  const onPath = searchDirs(pathDirs, "soli", resolveEnv, tried);
  if (onPath) {
    return { path: onPath, tried };
  }
  return { path: searchDirs(knownInstallDirs(resolveEnv), "soli", resolveEnv, tried), tried };
}
