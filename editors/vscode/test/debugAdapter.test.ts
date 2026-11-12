import * as assert from "node:assert/strict";
import { test } from "node:test";
import { adapterFor } from "../src/debugAdapter";

test("a launch runs `soli dap` with the resolved binary", () => {
  assert.deepEqual(adapterFor({ request: "launch" }, "/usr/local/bin/soli"), {
    kind: "executable",
    command: "/usr/local/bin/soli",
    args: ["dap"],
  });
});

test("a launch without a soli binary says so", () => {
  const choice = adapterFor({ request: "launch" }, undefined);
  assert.equal(choice.kind, "error");
});

test("an attach connects to the --inspect port, 9229 by default", () => {
  assert.deepEqual(adapterFor({ request: "attach" }, undefined), {
    kind: "server",
    host: "127.0.0.1",
    port: 9229,
  });
  assert.deepEqual(adapterFor({ request: "attach", port: 9339, host: "localhost" }, undefined), {
    kind: "server",
    host: "localhost",
    port: 9339,
  });
});

test("a bad attach port is refused", () => {
  assert.equal(adapterFor({ request: "attach", port: "9229" }, undefined).kind, "error");
  assert.equal(adapterFor({ request: "attach", port: 70000 }, undefined).kind, "error");
});
