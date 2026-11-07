// Bundles the extension into out/extension.js (vscode-languageclient
// included), and with --tests the pure modules and their tests into
// out-test/ for `node --test`.
import * as esbuild from "esbuild";
import { readdirSync } from "node:fs";

const args = new Set(process.argv.slice(2));
const production = args.has("--production");

if (args.has("--tests")) {
  const entryPoints = readdirSync("test")
    .filter((file) => file.endsWith(".test.ts"))
    .map((file) => `test/${file}`);
  await esbuild.build({
    entryPoints,
    outdir: "out-test",
    bundle: true,
    platform: "node",
    format: "cjs",
    target: "node20",
    // Tests run under plain Node: they must not reach the editor API.
    external: ["vscode", "vscode-textmate", "vscode-oniguruma"],
    logLevel: "warning",
  });
} else {
  const options = {
    entryPoints: ["src/extension.ts"],
    outfile: "out/extension.js",
    bundle: true,
    platform: "node",
    format: "cjs",
    target: "node18",
    external: ["vscode"],
    minify: production,
    sourcemap: !production,
    logLevel: "info",
  };
  if (args.has("--watch")) {
    const context = await esbuild.context(options);
    await context.watch();
  } else {
    await esbuild.build(options);
  }
}
