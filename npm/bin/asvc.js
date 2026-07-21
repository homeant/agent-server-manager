#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { createRequire } from "node:module";
import path from "node:path";

const require = createRequire(import.meta.url);

function resolveBinary() {
  if (process.env.ASVC_BINARY_PATH) return process.env.ASVC_BINARY_PATH;

  const key = `${process.platform}-${process.arch}`;
  const packages = {
    "darwin-arm64": "@homeant/asvc-darwin-arm64",
    "darwin-x64": "@homeant/asvc-darwin-x64",
    "linux-arm64": "@homeant/asvc-linux-arm64",
    "linux-x64": "@homeant/asvc-linux-x64",
    "win32-x64": "@homeant/asvc-win32-x64",
  };
  const packageName = packages[key];
  if (!packageName) {
    throw new Error(`asvc 暂不支持 ${key}`);
  }
  let packageJson;
  try {
    packageJson = require.resolve(`${packageName}/package.json`);
  } catch {
    throw new Error(
      `缺少 ${packageName}。请确认 npm 没有使用 --omit=optional，并重新安装 @homeant/asvc。`
    );
  }
  return path.join(
    path.dirname(packageJson),
    "bin",
    process.platform === "win32" ? "asvc.exe" : "asvc"
  );
}

try {
  const result = spawnSync(resolveBinary(), process.argv.slice(2), {
    stdio: "inherit",
    env: process.env,
  });
  if (result.error) throw result.error;
  if (result.signal) {
    process.kill(process.pid, result.signal);
  } else {
    process.exit(result.status ?? 1);
  }
} catch (error) {
  console.error(`asvc: ${error instanceof Error ? error.message : String(error)}`);
  process.exit(1);
}
