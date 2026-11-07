import * as assert from "node:assert/strict";
import { test } from "node:test";
import { knownInstallDirs, ResolveEnv, resolveSoli } from "../src/resolveBinary";

function posixEnv(existing: string[], PATH = "/usr/bin:/bin"): ResolveEnv {
  return {
    platform: "linux",
    env: { PATH },
    homedir: "/home/ada",
    isExecutable: (file) => existing.includes(file),
  };
}

function windowsEnv(existing: string[], Path = "C:\\Windows\\System32"): ResolveEnv {
  return {
    platform: "win32",
    env: { Path, PATHEXT: ".COM;.EXE;.BAT;.CMD" },
    homedir: "C:\\Users\\ada",
    isExecutable: (file) => existing.includes(file),
  };
}

test("the configured path wins", () => {
  const resolved = resolveSoli("/opt/soli/bin/soli", posixEnv(["/opt/soli/bin/soli", "/usr/bin/soli"]));
  assert.equal(resolved.path, "/opt/soli/bin/soli");
});

test("a configured path that does not exist is reported, not replaced by another soli", () => {
  const resolved = resolveSoli("/nowhere/soli", posixEnv(["/usr/bin/soli"]));
  assert.equal(resolved.path, undefined);
  assert.deepEqual(resolved.tried, ["/nowhere/soli"]);
});

test("a configured ~/ path is expanded", () => {
  const resolved = resolveSoli("~/bin/soli", posixEnv(["/home/ada/bin/soli"]));
  assert.equal(resolved.path, "/home/ada/bin/soli");
});

test("a configured bare name is looked up on PATH (the 0.2 default, \"soli\")", () => {
  const resolved = resolveSoli("soli", posixEnv(["/bin/soli"]));
  assert.equal(resolved.path, "/bin/soli");
});

test("PATH comes before the known install directories", () => {
  const resolved = resolveSoli("", posixEnv(["/bin/soli", "/home/ada/.local/bin/soli"]));
  assert.equal(resolved.path, "/bin/soli");
});

test("without soli on PATH, ~/.local/bin, ~/.cargo/bin and /usr/local/bin are tried in order", () => {
  const resolved = resolveSoli("", posixEnv(["/home/ada/.cargo/bin/soli", "/usr/local/bin/soli"]));
  assert.equal(resolved.path, "/home/ada/.cargo/bin/soli");
  assert.deepEqual(resolved.tried, [
    "/usr/bin/soli",
    "/bin/soli",
    "/home/ada/.local/bin/soli",
    "/home/ada/.cargo/bin/soli",
  ]);
});

test("nothing found lists every place looked at", () => {
  const resolved = resolveSoli(undefined, posixEnv([]));
  assert.equal(resolved.path, undefined);
  assert.ok(resolved.tried.includes("/usr/local/bin/soli"));
  assert.ok(resolved.tried.includes("/home/ada/.local/bin/soli"));
});

test("CARGO_HOME moves the cargo bin directory", () => {
  const env = posixEnv([]);
  env.env.CARGO_HOME = "/data/cargo";
  assert.ok(knownInstallDirs(env).includes("/data/cargo/bin"));
});

test("on Windows, soli.exe is found on PATH", () => {
  const resolved = resolveSoli("", windowsEnv(["C:\\tools\\soli.exe"], "C:\\Windows;C:\\tools"));
  assert.equal(resolved.path, "C:\\tools\\soli.exe");
});

test("on Windows, %USERPROFILE%\\.cargo\\bin\\soli.exe is a known location", () => {
  const resolved = resolveSoli("", windowsEnv(["C:\\Users\\ada\\.cargo\\bin\\soli.exe"]));
  assert.equal(resolved.path, "C:\\Users\\ada\\.cargo\\bin\\soli.exe");
});

test("on Windows, a configured path without .exe still finds soli.exe", () => {
  const resolved = resolveSoli("D:\\soli\\soli", windowsEnv(["D:\\soli\\soli.exe"]));
  assert.equal(resolved.path, "D:\\soli\\soli.exe");
});
