import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const root = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const binariesDir = path.join(root, "src-tauri", "binaries");
const executable = process.platform === "win32" ? "asvc.exe" : "asvc";

const targetForHost = {
  "darwin-arm64": "aarch64-apple-darwin",
  "darwin-x64": "x86_64-apple-darwin",
  "linux-arm64": "aarch64-unknown-linux-gnu",
  "linux-x64": "x86_64-unknown-linux-gnu",
  "win32-x64": "x86_64-pc-windows-msvc",
};

const platform = `${process.platform}-${process.arch}`;
const tauriTarget = process.env.ASVC_TAURI_TARGET || targetForHost[platform];
if (!tauriTarget) {
  throw new Error(`unsupported desktop packaging platform: ${platform}`);
}

const sourceTarget = process.env.ASVC_BUILD_TARGET;
const source = sourceTarget
  ? path.join(root, "target", sourceTarget, "release", executable)
  : path.join(root, "target", "release", executable);
if (!fs.existsSync(source)) {
  throw new Error(
    `missing headless CLI at ${source}; run cargo build --release --locked first`
  );
}

fs.mkdirSync(binariesDir, { recursive: true });
for (const entry of fs.readdirSync(binariesDir)) {
  if (entry === "asvc" || entry.startsWith("asvc-")) {
    fs.rmSync(path.join(binariesDir, entry), { force: true });
  }
}

const targetBinary = path.join(
  binariesDir,
  `asvc-${tauriTarget}${process.platform === "win32" ? ".exe" : ""}`
);
fs.copyFileSync(source, targetBinary);
// Keep an unqualified copy for local inspection and packaging checks. Tauri's externalBin
// lookup uses the suffixed copy and places it next to the desktop executable in each bundle.
const packageBinary = path.join(binariesDir, executable);
fs.copyFileSync(source, packageBinary);
if (process.platform !== "win32") {
  fs.chmodSync(targetBinary, 0o755);
  fs.chmodSync(packageBinary, 0o755);
}

console.log(`desktop CLI sidecar: ${targetBinary}`);
console.log(`desktop package CLI: ${packageBinary}`);
