import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const pkg = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8"));
const launcher = path.join(root, "npm", "bin", "asvc.js");
const platform = `${process.platform}-${process.arch}`;
const executable = process.platform === "win32" ? "asvc.exe" : "asvc";
const binary = path.join(root, "release", platform, executable);
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-npm-launcher-"));
const home = path.join(fixture, "home");
const asvcHome = path.join(home, ".asvc");
fs.mkdirSync(home, { recursive: true });

const env = {
  ...process.env,
  HOME: home,
  USERPROFILE: home,
  ASVC_HOME: asvcHome,
  ASVC_BINARY_PATH: binary,
  PATH:
    process.platform === "win32"
      ? [path.dirname(process.execPath), path.join(process.env.SystemRoot, "System32")].join(
          path.delimiter
        )
      : "/usr/bin:/bin",
};

function run(args) {
  const result = spawnSync(process.execPath, [launcher, ...args], {
    env,
    encoding: "utf8",
    timeout: 15_000,
  });
  assert.equal(result.status, 0, `${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

try {
  assert.equal(run(["--version"]).trim(), pkg.version);
  assert.match(run(["daemon", "status"]), /daemon 未运行/);
  assert.match(run(["list"]), /暂无服务/);
  assert.match(run(["daemon", "status"]), /daemon 运行中/);
  assert.match(run(["daemon", "stop"]), /daemon 已停止/);
  console.log("npm launcher smoke test passed");
} finally {
  spawnSync(binary, ["daemon", "stop"], { env, encoding: "utf8", timeout: 5_000 });
  fs.rmSync(fixture, { recursive: true, force: true });
}
