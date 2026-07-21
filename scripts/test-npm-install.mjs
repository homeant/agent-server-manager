import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { spawnSync } from "node:child_process";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const platform = `${process.platform}-${process.arch}`;
const executable = process.platform === "win32" ? "asvc.exe" : "asvc";
const fixture = fs.mkdtempSync(path.join(os.tmpdir(), "asvc-npm-install-"));
const packDir = path.join(fixture, "packs");
const prefix = path.join(fixture, "prefix");
const home = path.join(fixture, "home");
fs.mkdirSync(packDir, { recursive: true });
fs.mkdirSync(home, { recursive: true });

const npmCli = process.env.npm_execpath;
assert.ok(npmCli, "test must run through npm so npm_execpath is available");

function npm(args, cwd = root) {
  const result = spawnSync(process.execPath, [npmCli, ...args], {
    cwd,
    env: process.env,
    encoding: "utf8",
    timeout: 60_000,
  });
  assert.equal(result.status, 0, `npm ${args.join(" ")}\n${result.stdout}\n${result.stderr}`);
  return result.stdout;
}

function command(executable, args, env) {
  const result = spawnSync(executable, args, {
    env,
    encoding: "utf8",
    timeout: 15_000,
    shell: process.platform === "win32" && executable.toLowerCase().endsWith(".cmd"),
  });
  assert.equal(
    result.status,
    0,
    `${executable} ${args.join(" ")}\n${result.stdout}\n${result.stderr}`
  );
  return result.stdout;
}

try {
  const platformPack = JSON.parse(
    npm(
      ["pack", "--json", "--pack-destination", packDir],
      path.join(root, "npm", `asvc-${platform}`)
    )
  )[0].filename;
  const rootPack = JSON.parse(
    npm(["pack", "--json", "--pack-destination", packDir], root)
  )[0].filename;
  npm([
    "install",
    "--global",
    "--prefix",
    prefix,
    "--ignore-scripts",
    "--no-audit",
    path.join(packDir, platformPack),
    path.join(packDir, rootPack),
  ]);

  const asvc =
    process.platform === "win32"
      ? path.join(prefix, "asvc.cmd")
      : path.join(prefix, "bin", "asvc");
  const asvcHome = path.join(home, ".asvc");
  const env = {
    ...process.env,
    HOME: home,
    USERPROFILE: home,
    ASVC_HOME: asvcHome,
    PATH:
      process.platform === "win32"
        ? [prefix, path.dirname(process.execPath), path.join(process.env.SystemRoot, "System32")].join(
            path.delimiter
          )
        : [path.join(prefix, "bin"), path.dirname(process.execPath), "/usr/bin", "/bin"].join(
            path.delimiter
          ),
  };
  const version = JSON.parse(fs.readFileSync(path.join(root, "package.json"), "utf8")).version;
  assert.equal(command(asvc, ["--version"], env).trim(), version);
  assert.match(command(asvc, ["daemon", "status"], env), /daemon 未运行/);
  assert.match(command(asvc, ["list"], env), /暂无服务/);

  const daemonPid = fs.readFileSync(path.join(asvcHome, "daemon.pid"), "utf8").trim();
  if (process.platform === "win32") {
    assert.match(
      command("tasklist.exe", ["/FI", `PID eq ${daemonPid}`, "/FO", "CSV", "/NH"], env),
      /asvc\.exe/i
    );
  } else {
    const daemonCommand = command("/bin/ps", ["-p", daemonPid, "-o", "command="], env);
    assert.match(daemonCommand, new RegExp(`asvc-${platform}/bin/${executable} __daemon`));
  }
  assert.match(command(asvc, ["daemon", "stop"], env), /daemon 已停止/);
  console.log("packed npm install smoke test passed");
} finally {
  fs.rmSync(fixture, { recursive: true, force: true });
}
